"""translator 的 provider 選擇與錯誤映射測試（mock client，不打真 API）。
兩種後端都走串流：假 client 回 SSE 行（OpenAI 相容）或假的 MessageStream（Claude），
取消測試用「卡住直到被 close」的回應模擬模型還在生成時客戶端主動斷線。"""
import json
import threading
from contextlib import contextmanager

import anthropic
import httpx
import httpx2
import pytest

from src.translation.examples import (
    EMPTY,
    GAME_LANGUAGE_EXAMPLES,
    ExampleSet,
    ExampleStore,
    examples_fingerprint,
)
from src.translation.postprocess import (
    has_stray_latin,
    number_lines,
    restore_sender,
    tidy_parentheses,
    unnumber_lines,
)
from src.translation.prompts import (
    OUTGOING_LANGUAGE,
    PROMPT_REVISION,
    _game_noun_rule,
    build_incoming_system,
    build_region_system,
    build_system_message_system,
    example_turns,
)
from src.translation.translator import (
    _MAX_TOKENS_REGION,
    _MAX_TOKENS_THINKING,
    OPENAI_BASE_URL,
    RequestHandle,
    Translator,
    TranslatorBadOutput,
    TranslatorCancelled,
    TranslatorConfigError,
    TranslatorNoModelList,
    TranslatorOffline,
    error_detail,
    generate_and_store,
    list_models,
)
from src.translation.translator import test_translate as run_test_translate


def _sse(chunks) -> list[str]:
    return [f"data: {json.dumps(c, ensure_ascii=False)}" for c in chunks] + ["data: [DONE]"]


class FakeResponse:
    """替身串流回應：`iter_lines` 把 content 逐字拆成 SSE 塊送出（證明客戶端有把塊接回去），
    finish_reason 跟在最後一塊、usage（若有）另成一塊，與 OpenAI 的串流格式一致。
    `json()` 只給 list_models 的 GET 用。"""

    def __init__(self, status_code=200, content="譯文", finish_reason="stop",
                 completion_tokens=None, payload=None, text=""):
        self.status_code = status_code
        self.text = text
        self.closed = False
        self._content = content
        self._finish_reason = finish_reason
        self._completion_tokens = completion_tokens
        self._payload = payload

    def iter_lines(self):
        pieces = list(self._content) or [""]
        chunks = [{"choices": [{"delta": {"content": piece}, "finish_reason": None}]}
                  for piece in pieces]
        chunks[-1]["choices"][0]["finish_reason"] = self._finish_reason
        if self._completion_tokens is not None:
            chunks.append({"choices": [], "usage": {"completion_tokens": self._completion_tokens}})
        yield from _sse(chunks)

    def read(self):
        return self.text.encode("utf-8")

    def close(self):
        self.closed = True

    def json(self):
        if self._payload is not None:
            return self._payload
        data = {"choices": [{"message": {"content": self._content},
                             "finish_reason": self._finish_reason}]}
        if self._completion_tokens is not None:
            data["usage"] = {"completion_tokens": self._completion_tokens}
        return data

    def raise_for_status(self):
        if self.status_code >= 400:
            raise httpx.HTTPStatusError("err", request=None, response=None)


class FakeHttpxClient:
    """替身 httpx.Client：stream 回傳預設回應或拋出預設例外。"""
    def __init__(self, response=None, raises=None):
        self._response = response or FakeResponse()
        self._raises = raises
        self.last_body = None

    @contextmanager
    def stream(self, method, url, json):
        assert method == "POST"
        self.last_body = json
        if self._raises:
            raise self._raises
        yield self._response

    def get(self, url):
        self.last_url = url
        if self._raises:
            raise self._raises
        return self._response


def _make(client, provider="custom"):
    return Translator(provider=provider, base_url="http://x", model="m",
                      target_language="繁體中文（台灣）", client=client)


def test_openai_compat_sends_temperature_zero():
    fake = FakeHttpxClient()
    assert _make(fake).translate_outgoing("哈囉", []) == "譯文"
    assert fake.last_body["temperature"] == 0


def test_openai_compat_connection_error_maps_to_offline():
    fake = FakeHttpxClient(raises=httpx.ConnectError("refused"))
    with pytest.raises(TranslatorOffline):
        _make(fake).translate_incoming("[A] hi", [])


@pytest.mark.parametrize("status", [401, 403, 404])
def test_openai_compat_auth_or_model_error_maps_to_config_error(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorConfigError) as ei:
        _make(fake).translate_incoming("[A] hi", [])
    assert ei.value.status == status


@pytest.mark.parametrize("status", [429, 500, 503])
def test_openai_compat_retryable_status_maps_to_offline(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorOffline):
        _make(fake).translate_incoming("[A] hi", [])


class FakeMessageStream:
    """替身 MessageStream：`get_final_message` 回累積好的訊息。"""

    def __init__(self, stop_reason, content):
        self._stop_reason, self._content = stop_reason, content
        self.closed = False

    def get_final_message(self):
        stop_reason = self._stop_reason
        content = self._content

        class Block:
            type = "text"
            text = content

        class Resp:
            content = [Block()]

        Resp.stop_reason = stop_reason
        return Resp()

    def close(self):
        self.closed = True


class FakeAnthropicMessages:
    def __init__(self, raises=None, stop_reason="end_turn", content="克勞德譯文"):
        self._raises = raises
        self._stop_reason = stop_reason
        self._content = content
        self.last_kwargs = None
        self.last_stream = None

    @contextmanager
    def stream(self, **kwargs):
        self.last_kwargs = kwargs
        if self._raises:
            raise self._raises
        self.last_stream = FakeMessageStream(self._stop_reason, self._content)
        yield self.last_stream


class FakeAnthropicClient:
    def __init__(self, raises=None, stop_reason="end_turn", content="克勞德譯文"):
        self.messages = FakeAnthropicMessages(raises, stop_reason, content)


def _anthropic_status_error(status, body=None):
    resp = httpx2.Response(status, request=httpx2.Request("POST", "http://x"))
    return anthropic.APIStatusError("err", response=resp, body=body)


def test_claude_provider_returns_text():
    t = Translator(provider="claude", model="claude-opus-5", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient())
    assert t.translate_incoming("[A] hi", []) == "[A] 克勞德譯文"


def test_claude_connection_error_maps_to_offline():
    err = anthropic.APIConnectionError(request=httpx2.Request("POST", "http://x"))
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient(raises=err))
    with pytest.raises(TranslatorOffline):
        t.translate_incoming("[A] hi", [])


@pytest.mark.parametrize("status,exc", [(401, TranslatorConfigError),
                                        (404, TranslatorConfigError),
                                        (429, TranslatorOffline),
                                        (500, TranslatorOffline)])
def test_claude_status_error_mapping(status, exc):
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）",
                   client=FakeAnthropicClient(raises=_anthropic_status_error(status)))
    with pytest.raises(exc):
        t.translate_incoming("[A] hi", [])


def test_openai_compat_sends_max_tokens_by_thinking_mode():
    # 無上限時模型 repetition loop 會生成到吃穿 timeout；思考模式需放寬讓 think 區塊放得下
    from src.translation.translator import _MAX_TOKENS
    off = FakeHttpxClient()
    Translator(provider="custom", base_url="http://x", model="m", thinking=False,
               target_language="繁體中文（台灣）", client=off).translate_incoming("[A] hi", [])
    assert off.last_body["max_tokens"] == _MAX_TOKENS
    on = FakeHttpxClient()
    Translator(provider="custom", base_url="http://x", model="m", thinking=True,
               target_language="繁體中文（台灣）", client=on).translate_incoming("[A] hi", [])
    assert on.last_body["max_tokens"] == _MAX_TOKENS_THINKING


def test_openai_compat_truncated_output_maps_to_bad_output():
    fake = FakeHttpxClient(response=FakeResponse(content="呃 呃 呃", finish_reason="length",
                                                 completion_tokens=7))
    with pytest.raises(TranslatorBadOutput) as ei:
        _make(fake).translate_incoming("[A] am chick um chick", [])
    # 診斷資訊要進得了 app.log：token 數與樣本用來分辨 repetition loop 與譯文真的過長
    message = str(ei.value)
    assert "completion_tokens=7" in message
    assert "呃 呃 呃" in message


def test_openai_compat_missing_finish_reason_is_accepted():
    # 部分後端不回 finish_reason，不得因此誤判為截斷
    fake = FakeHttpxClient(response=FakeResponse(finish_reason=None))
    assert _make(fake).translate_incoming("[A] hi", []) == "[A] 譯文"


def test_claude_sends_max_tokens():
    fake = FakeAnthropicClient()
    Translator(provider="claude", model="m", api_key="k",
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.messages.last_kwargs["max_tokens"] == _MAX_TOKENS_THINKING


def test_claude_auto_effort_sends_no_output_config():
    # 自動＝維持模型預設（adaptive），連參數都不帶
    fake = FakeAnthropicClient()
    Translator(provider="claude", model="m", api_key="k",
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert "output_config" not in fake.messages.last_kwargs


def test_claude_low_effort_sends_output_config():
    from src.services import EFFORT_LOW
    fake = FakeAnthropicClient()
    Translator(provider="claude", model="m", api_key="k", effort=EFFORT_LOW,
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.messages.last_kwargs["output_config"] == {"effort": "low"}
    # 思考深度壓低不代表關閉思考：Claude 沒有 thinking 開關可送
    assert "thinking" not in fake.messages.last_kwargs


def test_claude_truncated_output_maps_to_bad_output():
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）",
                   client=FakeAnthropicClient(stop_reason="max_tokens"))
    with pytest.raises(TranslatorBadOutput):
        t.translate_incoming("[A] hi", [])


def test_openai_provider_disables_thinking_with_official_param_only():
    # openai provider 關閉思考時只帶官方認得的 reasoning_effort，
    # 不得夾帶自架後端專用參數（官方端點對未知欄位嚴格回 400）。
    fake = FakeHttpxClient()
    t = Translator(provider="openai", model="m", api_key="k", thinking=False,
                   target_language="繁體中文（台灣）", client=fake)
    t.translate_incoming("[A] hi", [])
    assert fake.last_body["reasoning_effort"] == "none"
    for key in ("chat_template_kwargs", "think", "enable_thinking"):
        assert key not in fake.last_body


def test_openai_provider_thinking_on_sends_no_thinking_params():
    fake = FakeHttpxClient()
    t = Translator(provider="openai", model="m", api_key="k", thinking=True,
                   target_language="繁體中文（台灣）", client=fake)
    t.translate_incoming("[A] hi", [])
    for key in ("reasoning_effort", "chat_template_kwargs", "think", "enable_thinking"):
        assert key not in fake.last_body


def test_openai_provider_uses_max_completion_tokens_and_no_temperature():
    # 官方端點：max_tokens 已棄用、GPT-5／o 系列直接回 400「use max_completion_tokens」；
    # 同一批模型也只接受預設 temperature（實測「Only the default (1) value is supported」）
    from src.translation.translator import _MAX_TOKENS

    fake = FakeHttpxClient()
    t = Translator(provider="openai", model="m", api_key="k", thinking=False,
                   target_language="繁體中文（台灣）", client=fake)
    t.translate_incoming("[A] hi", [])
    assert fake.last_body["max_completion_tokens"] == _MAX_TOKENS
    assert "max_tokens" not in fake.last_body
    assert "temperature" not in fake.last_body
    fake = FakeHttpxClient()
    Translator(provider="openai", model="m", api_key="k", thinking=True,
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.last_body["max_completion_tokens"] == _MAX_TOKENS_THINKING


def test_custom_endpoint_keeps_max_tokens_and_temperature():
    # 自架後端（vLLM／Ollama／LM Studio）多半只認 max_tokens，temperature=0 也是為了它們的重現性
    fake = FakeHttpxClient()
    _make(fake).translate_incoming("[A] hi", [])
    assert "max_tokens" in fake.last_body
    assert "max_completion_tokens" not in fake.last_body
    assert fake.last_body["temperature"] == 0


class SequenceClient(FakeHttpxClient):
    """替身 httpx.Client：依序回傳 responses、用完重複最後一個，並記下每次送出的 body
    （參數重送與重譯路徑都要看多次請求）。"""
    def __init__(self, *responses):
        super().__init__()
        self._responses = list(responses)
        self.bodies = []

    @contextmanager
    def stream(self, method, url, json):
        self.bodies.append(json)
        yield self._responses.pop(0) if len(self._responses) > 1 else self._responses[0]


def _rejects(param: str, wording: str = "unrecognized"):
    messages = {
        "unrecognized": f"Unrecognized request argument supplied: {param}",
        "unsupported": f"Unsupported parameter: '{param}' is not supported with this model. "
                       f"Use 'max_completion_tokens' instead.",
        "value": f"Unsupported value: '{param}' does not support 0 with this model. "
                 f"Only the default (1) value is supported.",
    }
    return FakeResponse(status_code=400,
                        text=json.dumps({"error": {"message": messages[wording]}}))


def test_rejected_parameter_is_dropped_and_the_request_retried():
    # gpt-4o-mini 這類非推理模型不認 reasoning_effort：規則因模型而異、靠名稱猜會一直追不上，
    # 改成聽伺服器的 —— 它指名哪個參數就拿掉哪個重送
    fake = SequenceClient(_rejects("reasoning_effort"), FakeResponse())
    t = Translator(provider="openai", model="gpt-4o-mini", api_key="k", thinking=False,
                   target_language="繁體中文（台灣）", client=fake)
    assert t.translate_incoming("[A] hi", []) == "[A] 譯文"
    assert "reasoning_effort" in fake.bodies[0]
    assert "reasoning_effort" not in fake.bodies[1]
    # 學到的規則記在 client 上：下一句不再多送一次被拒的請求
    fake._responses.append(FakeResponse())
    t.translate_incoming("[A] again", [])
    assert len(fake.bodies) == 3 and "reasoning_effort" not in fake.bodies[2]


@pytest.mark.parametrize("wording", ["unsupported", "value"])
def test_other_openai_rejection_wordings_are_recognised(wording):
    fake = SequenceClient(_rejects("temperature", wording), FakeResponse())
    assert _make(fake).translate_incoming("[A] hi", []) == "[A] 譯文"
    assert "temperature" not in fake.bodies[1]


def test_rejected_token_limit_is_swapped_not_dropped():
    # 長度上限是防 repetition loop 的保險，不能因為端點只認另一個名字就整個不帶
    fake = SequenceClient(_rejects("max_completion_tokens"), FakeResponse())
    t = Translator(provider="openai", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=fake)
    t.translate_incoming("[A] hi", [])
    assert "max_completion_tokens" not in fake.bodies[1]
    assert fake.bodies[1]["max_tokens"] == _MAX_TOKENS_THINKING
    fake = SequenceClient(_rejects("max_tokens", "unsupported"), FakeResponse())
    _make(fake).translate_incoming("[A] hi", [])
    assert "max_tokens" not in fake.bodies[1]
    assert fake.bodies[1]["max_completion_tokens"] == _MAX_TOKENS_THINKING


def test_400_without_a_named_parameter_is_still_a_config_error():
    fake = SequenceClient(FakeResponse(status_code=400,
                                       text='{"error": {"message": "bad request"}}'))
    with pytest.raises(TranslatorConfigError) as ei:
        _make(fake).translate_incoming("[A] hi", [])
    assert ei.value.detail == "bad request"
    assert len(fake.bodies) == 1


def test_rejection_of_a_parameter_we_did_not_send_is_not_retried_forever():
    # 伺服器指名的參數本來就不在 body 裡：拿掉也沒用，照一般 400 處理
    fake = SequenceClient(_rejects("frequency_penalty"))
    with pytest.raises(TranslatorConfigError):
        _make(fake).translate_incoming("[A] hi", [])
    assert len(fake.bodies) == 1


def test_openai_provider_forces_official_base_url():
    t = Translator(provider="openai", base_url="http://evil.example", model="m",
                   api_key="k", target_language="繁體中文（台灣）")
    assert str(t._binding.impl._client.base_url) == OPENAI_BASE_URL


def test_build_turns_without_context_is_single_user_turn():
    from src.translation.prompts import build_turns
    assert build_turns([], "[A] hi", "intro") == [
        {"role": "user", "content": "[A] hi"}]


def test_build_turns_prepends_fewshot_examples():
    from src.translation.prompts import build_turns
    examples = [{"role": "user", "content": "在嗎"},
                {"role": "assistant", "content": "you there?"}]
    turns = build_turns([], "哈囉", "intro", examples=examples)
    assert turns[:2] == examples                       # 範例在最前
    assert turns[-1] == {"role": "user", "content": "哈囉"}  # 待翻句仍在最後


def test_outgoing_uses_fewshot_when_no_context():
    from src.translation.prompts import FEWSHOT_OUTGOING
    fake = FakeHttpxClient()
    _make(fake).translate_outgoing("在嗎", [])   # 無背景上下文：帶 few-shot 強制翻譯模式
    turns = _turns(fake.last_body)
    assert turns[:len(FEWSHOT_OUTGOING)] == FEWSHOT_OUTGOING
    assert turns[-1] == {"role": "user", "content": "在嗎"}
    assert any("提供" in m["content"] for m in FEWSHOT_OUTGOING if m["role"] == "user")


def test_incoming_uses_given_context():
    fake = FakeHttpxClient()
    _make(fake).translate_incoming("[B] two", ["[A] one"])
    turns = _turns(fake.last_body)
    assert len(turns) == 3                      # user(背景)+assistant(ack)+user(待翻)
    assert "[A] one" in turns[0]["content"]
    assert turns[-1] == {"role": "user", "content": "[B] two"}


def test_incoming_without_context_is_single_turn():
    fake = FakeHttpxClient()
    _make(fake).translate_incoming("[A] one", [])
    assert _turns(fake.last_body) == [{"role": "user", "content": "[A] one"}]


def test_translator_keeps_no_internal_history():
    # 上下文改由呼叫端（ChatContext）維護：translator 連續翻兩則也不得自行累積
    fake = FakeHttpxClient()
    t = _make(fake)
    t.translate_incoming("[A] one", [])
    t.translate_incoming("[B] two", [])
    assert _turns(fake.last_body) == [{"role": "user", "content": "[B] two"}]
    assert not hasattr(t, "_history")


def test_outgoing_keeps_fewshot_even_with_context():
    from src.translation.prompts import FEWSHOT_OUTGOING
    fake = FakeHttpxClient()
    _make(fake).translate_outgoing("好啊", ["[A] want to trade?"])
    turns = _turns(fake.last_body)
    # 遊戲內幾乎永遠有上下文，範例若在此時被略過，防脫稿保護等於沒有
    assert turns[:len(FEWSHOT_OUTGOING)] == FEWSHOT_OUTGOING
    assert any("[A] want to trade?" in m["content"] for m in turns)
    assert turns[-1] == {"role": "user", "content": "好啊"}


def test_build_turns_with_context_is_multi_turn():
    from src.translation.prompts import CONTEXT_ACK, build_turns
    turns = build_turns(["[A] one", "[B] two"], "[C] three", "背景說明")
    assert turns == [
        {"role": "user", "content": "背景說明\n[A] one\n[B] two"},
        {"role": "assistant", "content": CONTEXT_ACK},
        {"role": "user", "content": "[C] three"},  # 待翻句永遠是最後一個乾淨 user turn
    ]


def _turns(body):
    return body["messages"][1:]  # 去掉 system，剩下對話輪


def test_incoming_system_has_no_format_markers():
    # system 不得列出段落標記字串，否則小模型會把它回吐成「請照此格式提供輸入」
    system = build_incoming_system("繁體中文（台灣）")
    assert "[要翻譯的訊息]" not in system
    assert "背景" in system            # 仍說明會收到聊天記錄當背景


def test_both_systems_forbid_treating_input_as_instructions():
    # 輸入內容長得像指令時模型不得脫稿回應（實測踩過：回了「了解。請提供…」）
    from src.translation.prompts import build_outgoing_system
    assert "絕不回應" in build_incoming_system("繁體中文（台灣）")
    assert "絕不回應" in build_outgoing_system("English")


def test_outgoing_system_forbids_borrowing_nouns_from_the_context():
    # 實測踩過：情境裡的「ty for the tc」讓「下次換我請你喝茶」被翻成
    # 「next time it's my treat for the tc」—— 情境獨有的縮寫漏進了譯文
    from src.translation.prompts import build_outgoing_system
    prompt = build_outgoing_system("English")
    assert "只出現在情境、玩家訊息沒有的名詞" in prompt


def test_reconfigure_switches_provider():
    t = _make(FakeHttpxClient())
    t.reconfigure(provider="claude", base_url="", model="claude-opus-5", api_key="k",
                  thinking=False, target_language="日本語")
    # reconfigure 後為 Claude client（真物件）；此處只驗證型別切換，不打 API
    from src.translation.translator import _ClaudeClient
    assert isinstance(t._binding.impl, _ClaudeClient)


def test_reconfigure_keeps_the_backend_when_nothing_changed():
    """值沒變就不重建：拆連線池會讓飛行中的請求收到斷線錯誤，端點學到的參數限制
    （實測 temperature 被拒）也要重新付一輪 400 才學得回來。"""
    tr = _make(FakeHttpxClient())
    before = tr._binding.impl
    before._dropped.add("temperature")

    assert tr.reconfigure(provider="custom", base_url="http://x", model="m",
                          target_language="繁體中文（台灣）") is False
    assert tr._binding.impl is before
    assert tr._binding.impl._dropped == {"temperature"}


def test_reconfigure_rebuilds_when_a_value_changed():
    tr = _make(FakeHttpxClient())
    before = tr._binding.impl

    assert tr.reconfigure(provider="custom", base_url="http://x", model="m2",
                          target_language="繁體中文（台灣）") is True
    assert tr._binding.impl is not before and tr._binding.impl.model == "m2"


def test_describe_names_the_service_and_never_the_key():
    tr = Translator(provider="openai", model="gpt-x", api_key="sk-secret",
                    target_language="繁體中文（台灣）", client=FakeHttpxClient())
    assert tr.describe() == "provider=openai, model=gpt-x"


class ReconfiguringClient(FakeHttpxClient):
    """請求進行中設定就被換掉（使用者按下儲存）；`translator` 由測試接上。"""

    translator = None

    @contextmanager
    def stream(self, method, url, json):
        with super().stream(method, url, json) as response:
            self.translator.reconfigure(provider="custom", base_url="http://x",
                                        model="m2", target_language="日本語")
            yield response


def test_the_success_log_names_the_model_that_served_the_request(capsys):
    # 請求中途重讀設定的話，期間的 reconfigure() 會讓這一行記成新模型
    fake = ReconfiguringClient()
    fake.translator = tr = _make(fake)
    tr.translate_incoming("[A] hi", [])
    err = capsys.readouterr().err
    assert "model=m," in err and "model=m2" not in err


class FakeModel:
    def __init__(self, model_id):
        self.id = model_id


class FakeAnthropicModels:
    def __init__(self, ids=(), raises=None):
        self._ids = ids
        self._raises = raises

    def list(self):
        if self._raises:
            raise self._raises
        return [FakeModel(i) for i in self._ids]


def _api(provider="custom", base_url="http://x", model="", api_key="k"):
    """設定表單當下的值（扁平）：每家欄位不同，這裡給的是自訂端點那組。"""
    api = {"provider": provider, "model": model, "api_key": api_key}
    if provider == "custom":
        api.update(base_url=base_url, thinking=False)
    return api


def test_list_models_openai_compat_returns_sorted_ids():
    fake = FakeHttpxClient(response=FakeResponse(
        payload={"data": [{"id": "qwen3"}, {"id": "gemma3"}]}))
    assert list_models(_api(), client=fake) == ["gemma3", "qwen3"]
    assert fake.last_url == "/v1/models"


@pytest.mark.parametrize("status", [404, 405])
def test_list_models_missing_endpoint_maps_to_no_model_list(status):
    # 端點沒有 /v1/models：與「模型不存在」是兩回事，不可映射成 TranslatorConfigError
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorNoModelList):
        list_models(_api(), client=fake)


def test_list_models_response_without_data_maps_to_no_model_list():
    fake = FakeHttpxClient(response=FakeResponse(payload={"object": "list"}))
    with pytest.raises(TranslatorNoModelList):
        list_models(_api(), client=fake)


@pytest.mark.parametrize("status", [401, 403])
def test_list_models_auth_error_maps_to_config_error(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorConfigError) as ei:
        list_models(_api(), client=fake)
    assert ei.value.status == status


def test_list_models_connection_error_maps_to_offline():
    fake = FakeHttpxClient(raises=httpx.ConnectError("refused"))
    with pytest.raises(TranslatorOffline):
        list_models(_api(), client=fake)


@pytest.mark.parametrize("status", [429, 500, 503])
def test_list_models_retryable_status_maps_to_offline(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorOffline):
        list_models(_api(), client=fake)


def test_list_models_claude_uses_models_api():
    fake = FakeAnthropicClient()
    fake.models = FakeAnthropicModels(ids=("claude-opus-5", "claude-haiku-4-5"))
    assert list_models(_api(provider="claude"), client=fake) == [
        "claude-haiku-4-5", "claude-opus-5"]


@pytest.mark.parametrize("status,exc", [(401, TranslatorConfigError),
                                        (404, TranslatorNoModelList),
                                        (429, TranslatorOffline),
                                        (500, TranslatorOffline)])
def test_list_models_claude_status_error_mapping(status, exc):
    fake = FakeAnthropicClient()
    fake.models = FakeAnthropicModels(raises=_anthropic_status_error(status))
    with pytest.raises(exc):
        list_models(_api(provider="claude"), client=fake)


def test_list_models_claude_connection_error_maps_to_offline():
    fake = FakeAnthropicClient()
    fake.models = FakeAnthropicModels(
        raises=anthropic.APIConnectionError(request=httpx2.Request("GET", "http://x")))
    with pytest.raises(TranslatorOffline):
        list_models(_api(provider="claude"), client=fake)


@pytest.mark.parametrize("text,expected", [
    ('{"error": {"message": "Incorrect API key provided: sk-abc***"}}',
     "Incorrect API key provided: sk-abc***"),
    ('{"error": "model not found"}', "model not found"),
    ('{"message": "quota exceeded"}', "quota exceeded"),
    ('  plain text \n body ', "plain text body"),
    ("<html><head><title>404 Not Found</title></head><body><h1>404 Not Found</h1>"
     "<hr><center>nginx</center></body></html>", "404 Not Found 404 Not Found nginx"),
    ("", ""),
    ('{"error": {"code": 42}}', '{"error": {"code": 42}}'),
])
def test_error_detail_extracts_api_message(text, expected):
    assert error_detail(text) == expected


def test_error_detail_keeps_the_whole_message():
    # 不截斷：訊息尾端常是說明網址，切掉就少了最有用的部分；只折成單行
    url = "https://platform.openai.com/account/api-keys"
    body = "word " * 60 + "\n  " + url + " tail"
    detail = error_detail(body)
    assert detail == "word " * 60 + url + " tail"
    assert "…" not in detail


def test_openai_compat_config_error_carries_api_message():
    fake = FakeHttpxClient(response=FakeResponse(
        status_code=401, text='{"error": {"message": "Incorrect API key provided"}}'))
    with pytest.raises(TranslatorConfigError) as ei:
        _make(fake).translate_incoming("[A] hi", [])
    assert ei.value.status == 401
    assert ei.value.detail == "Incorrect API key provided"
    assert str(ei.value) == "HTTP 401: Incorrect API key provided"


def test_openai_compat_other_4xx_maps_to_config_error():
    # 400 常是「模型不吃 temperature」這類設定問題，API 的說明必須帶出來
    fake = FakeHttpxClient(response=FakeResponse(
        status_code=400, text='{"error": {"message": "Unsupported parameter: temperature"}}'))
    with pytest.raises(TranslatorConfigError) as ei:
        _make(fake).translate_incoming("[A] hi", [])
    assert ei.value.status == 400
    assert ei.value.detail == "Unsupported parameter: temperature"


def test_openai_compat_offline_status_carries_api_message():
    fake = FakeHttpxClient(response=FakeResponse(status_code=503, text="upstream down"))
    with pytest.raises(TranslatorOffline) as ei:
        _make(fake).translate_incoming("[A] hi", [])
    assert ei.value.status == 503
    assert ei.value.detail == "upstream down"


def test_openai_compat_connection_error_carries_reason():
    fake = FakeHttpxClient(raises=httpx.ConnectError("[Errno 11001] getaddrinfo failed"))
    with pytest.raises(TranslatorOffline) as ei:
        _make(fake).translate_incoming("[A] hi", [])
    assert ei.value.status is None
    assert ei.value.detail == "[Errno 11001] getaddrinfo failed"


def test_claude_status_error_carries_api_message():
    body = {"type": "error", "error": {"type": "authentication_error",
                                       "message": "invalid x-api-key"}}
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）",
                   client=FakeAnthropicClient(raises=_anthropic_status_error(401, body)))
    with pytest.raises(TranslatorConfigError) as ei:
        t.translate_incoming("[A] hi", [])
    assert ei.value.detail == "invalid x-api-key"


def test_claude_other_4xx_maps_to_config_error():
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）",
                   client=FakeAnthropicClient(raises=_anthropic_status_error(400)))
    with pytest.raises(TranslatorConfigError) as ei:
        t.translate_incoming("[A] hi", [])
    assert ei.value.status == 400


def test_list_models_config_error_carries_api_message():
    fake = FakeHttpxClient(response=FakeResponse(
        status_code=403, text='{"error": {"message": "region not supported"}}'))
    with pytest.raises(TranslatorConfigError) as ei:
        list_models(_api(), client=fake)
    assert ei.value.detail == "region not supported"


def test_system_message_prompt_names_the_target_language():
    prompt = build_system_message_system("日本語")
    assert "日本語" in prompt


def test_system_message_prompt_does_not_mention_a_sender_prefix():
    # 系統訊息沒有 [發送者] 前綴，提示詞若照抄收訊那套會讓模型自己編一個出來
    prompt = build_system_message_system("繁體中文（台灣）")
    assert "[發送者]" not in prompt


def test_system_message_prompt_protects_placeholders():
    prompt = build_system_message_system("繁體中文（台灣）")
    assert "{0}" in prompt


def test_translate_system_message_sends_no_context_turns():
    fake = FakeHttpxClient()
    tr = Translator(target_language="繁體中文（台灣）", client=fake)
    tr.translate_system_message("你获得了 {0} 金币！")
    turns = fake.last_body["messages"][1:]  # 跳過 system message
    assert turns == [{"role": "user", "content": "你获得了 {0} 金币！"}]


def test_translate_system_message_strips_think_blocks():
    fake = FakeHttpxClient(response=FakeResponse(content="<think>hmm</think>你獲得了 {0} 金幣！"))
    tr = Translator(target_language="繁體中文（台灣）", client=fake)
    assert tr.translate_system_message("你获得了 {0} 金币！") == "你獲得了 {0} 金幣！"


# --- 遊戲名詞規則：括號裡的英文只能照抄原文既有的 ---
def test_game_noun_rule_is_shared_by_both_prompts():
    # 這條規則曾在收訊與系統訊息兩處各寫一份，改一處就會漏另一處 ——
    # 實機回報的「自行編造英文」正源於此。共用同一份，結構上防止再度分岔。
    rule = _game_noun_rule("日本語")
    assert rule in build_incoming_system("日本語")
    assert rule in build_system_message_system("日本語")


def test_game_noun_rule_forbids_inventing_english_for_non_english_source():
    # 原文非英文時模型不得自行翻一個英文塞進括號
    rule = _game_noun_rule("繁體中文（台灣）")
    assert "括號裡不得出現原文沒有的字串" in rule


def test_game_noun_rule_always_appends_the_original_for_every_target():
    # 使用者要求：收訊、系統訊息、區域的專有名詞一律附原文，目標是遊戲語言也一樣
    for language in ("繁體中文（台灣）", OUTGOING_LANGUAGE):
        assert "並在譯名後用括號逐字照抄原文寫法" in _game_noun_rule(language)


def test_system_prompts_no_longer_embed_an_example():
    for build in (build_incoming_system, build_system_message_system, build_region_system):
        prompt = build("日本語")
        assert "範例" not in prompt and "Fire Cat" not in prompt
        assert prompt.rstrip().endswith("譯文一律使用 日本語，不論原文或本說明是什麼語言。")


def test_example_turns_become_a_user_and_assistant_pair():
    assert example_turns(None) == []
    assert example_turns(("src", "out")) == [{"role": "user", "content": "src"},
                                             {"role": "assistant", "content": "out"}]


def test_prompt_revision_was_bumped_for_the_example_change():
    assert PROMPT_REVISION == 6


def test_game_noun_rule_keeps_player_and_npc_names_untranslated():
    assert "原樣保留" in _game_noun_rule("繁體中文（台灣）")


def test_game_noun_rule_names_the_target_language():
    assert "日本語" in _game_noun_rule("日本語")
    assert "Español" in _game_noun_rule("Español")


# --- tidy_parentheses：括號只能是照抄的原文，且不得重複前面的譯名 ---
def test_tidy_parentheses_removes_english_invented_for_a_cjk_source():
    # 實測：同一則簡中材料名，兩次翻譯分別補上 (Psychedelic Wood) 與 (Mystic Wood)
    assert tidy_parentheses("迷幻木头", "迷幻木頭(Mystic Wood)") == "迷幻木頭"


def test_tidy_parentheses_keeps_english_copied_from_the_source():
    assert tidy_parentheses("Proud Pegasus Statue", "驕傲的飛馬雕像(Proud Pegasus Statue)") \
        == "驕傲的飛馬雕像(Proud Pegasus Statue)"


def test_tidy_parentheses_removes_english_the_source_never_had():
    # 原文有別的英文也不放行：逐個括號比對，不再只看「原文有沒有英文」
    assert tidy_parentheses(
        "Talk to Bartleby at 天国大本营", "和 Bartleby 談談，地點在天國大本營（Heavenly HQ）"
    ) == "和 Bartleby 談談，地點在天國大本營"


def test_tidy_parentheses_ignores_an_english_sender_name():
    # 判斷只看訊息內容：發送者名是英文不代表內容有英文
    assert tidy_parentheses("[Amy] 迷幻木头", "[Amy] 迷幻木頭(Mystic Wood)") == "[Amy] 迷幻木頭"


def test_tidy_parentheses_handles_full_width_parentheses():
    assert tidy_parentheses("迷幻木头", "迷幻木頭（Mystic Wood）") == "迷幻木頭"


def test_tidy_parentheses_leaves_placeholders_alone():
    assert tidy_parentheses("你获得了 {0} 金币！", "你獲得了 {0} 金幣！") == "你獲得了 {0} 金幣！"


def test_tidy_parentheses_leaves_target_language_notes_alone():
    # 括號裡是譯文本身那種文字的說明就不是幻覺，原樣保留
    assert tidy_parentheses("熔岩百合", "熔岩百合（一種材料）") == "熔岩百合（一種材料）"
    assert tidy_parentheses("lava lily", "溶岩ユリ（素材の一つ）") == "溶岩ユリ（素材の一つ）"


def test_tidy_parentheses_removes_every_invented_occurrence():
    assert tidy_parentheses(
        "你获得了迷幻木头和熔岩百合", "你獲得了迷幻木頭(Mystic Wood)和熔岩百合(Lava Lily)"
    ) == "你獲得了迷幻木頭和熔岩百合"


def test_tidy_parentheses_removes_cjk_invented_for_a_latin_translation():
    # 實測（qwen，目標 English）：英文原文被仿照示範補上中文括號
    assert tidy_parentheses("do i use sirens", "do I use Sirens (塞壬)") == "do I use Sirens"


def test_tidy_parentheses_keeps_cjk_copied_from_the_source_for_a_latin_translation():
    assert tidy_parentheses("你获得了:风暴", "You gained: Storm (风暴)") == "You gained: Storm (风暴)"


def test_tidy_parentheses_keeps_accented_notes_for_a_latin_translation():
    # 帶重音的拉丁字母仍是譯文本身的文字，不能當成外文括號
    assert tidy_parentheses("hi", "hola (está aquí)") == "hola (está aquí)"


def test_tidy_parentheses_ignores_punctuation_when_matching_the_source():
    # 模型照抄時常換掉撇號或連字號，只比字母數字才不會把照抄的原文當成自創
    assert tidy_parentheses("Wizard’s Hat", "巫師帽（Wizard's Hat）") == "巫師帽（Wizard's Hat）"
    assert tidy_parentheses("ship of fools", "愚者之船（Ship-of-Fools）") == "愚者之船（Ship-of-Fools）"


def test_tidy_parentheses_keeps_parentheses_the_player_typed():
    # 原文本來就有的括號：譯成目標語言、或原樣照抄，兩種都要留著
    assert tidy_parentheses("[Amy] wts Fire Dragon (rank 5)", "[Amy] 出售火龍（Fire Dragon）（等級 5）") \
        == "[Amy] 出售火龍（Fire Dragon）（等級 5）"
    assert tidy_parentheses("[Amy] wts Fire Dragon (rank 5)", "[Amy] 出售火龍（Fire Dragon）(rank 5)") \
        == "[Amy] 出售火龍（Fire Dragon）(rank 5)"
    assert tidy_parentheses("[Amy] brb (dinner)", "[Amy] 等我一下（吃晚餐）") == "[Amy] 等我一下（吃晚餐）"


def test_tidy_parentheses_keeps_emoticons_built_from_parentheses():
    assert tidy_parentheses("[Amy] gg (╯°□°)╯", "[Amy] 好遊戲 (╯°□°)╯") == "[Amy] 好遊戲 (╯°□°)╯"
    assert tidy_parentheses("[Amy] hi (: ok :)", "[Amy] 嗨 (: ok :)") == "[Amy] 嗨 (: ok :)"


def test_tidy_parentheses_keeps_game_text_in_parentheses():
    # 框選與系統訊息常見的「(Rank 7)」「(x{0})」：出自原文就保留
    assert tidy_parentheses("Storm Lord (Rank 7)", "風暴領主（Storm Lord）(Rank 7)") \
        == "風暴領主（Storm Lord）(Rank 7)"
    assert tidy_parentheses("你获得了 鱼鳞 (x{0})", "你獲得了 魚鱗 (x{0})") == "你獲得了 魚鱗 (x{0})"


def test_tidy_parentheses_keeps_a_line_that_is_only_parentheses():
    # 實測（框選）：整行都是括號時括號外沒有字可判斷文字，翻好的「（需要等級 45）」曾被當成外文刪光
    assert tidy_parentheses("(Requires Level 45)", "（需要等級 45）") == "（需要等級 45）"
    assert tidy_parentheses("(Requires Level 45)", "(Requires Level 45)") == "(Requires Level 45)"


def test_tidy_parentheses_removes_an_original_that_only_echoes_the_name():
    # 實測：名詞沒翻就在後面重複一次（Malistaire（Malistaire）、Novus (Novus)）
    assert tidy_parentheses("farm Malistaire", "刷 Malistaire（Malistaire）") == "刷 Malistaire"
    assert tidy_parentheses("help me in novus", "help me in Novus (Novus)") == "help me in Novus"


def test_tidy_parentheses_keeps_a_repeated_number_the_player_typed():
    assert tidy_parentheses("[Amy] lvl 50 (50)", "[Amy] 等級 50（50）") == "[Amy] 等級 50（50）"


def test_translate_incoming_strips_invented_english():
    fake = FakeHttpxClient(response=FakeResponse(content="[艾米] 迷幻木頭(Mystic Wood)"))
    assert _make(fake).translate_incoming("[艾米] 迷幻木头", []) == "[艾米] 迷幻木頭"


def test_translate_system_message_strips_invented_english():
    fake = FakeHttpxClient(response=FakeResponse(content="迷幻木頭(Mystic Wood)"))
    assert _make(fake).translate_system_message("迷幻木头") == "迷幻木頭"


def _contents(*contents):
    """依序回這些譯文的 SequenceClient。"""
    return SequenceClient(*(FakeResponse(content=c) for c in contents))


# --- restore_sender：發送者名一律以原文為準，不交給模型 ---
def test_restore_sender_replaces_a_sender_the_model_rewrote():
    # 實測：漢化包的簡中玩家名被模型改字（贾斯廷 渡鸦 → 賈斯汀 渡鴉）
    assert restore_sender("[贾斯廷 渡鸦] hi", "[賈斯汀 渡鴉] 嗨") == "[贾斯廷 渡鸦] 嗨"


def test_restore_sender_puts_back_a_sender_the_model_dropped():
    assert restore_sender("[Amy] hi", "嗨") == "[Amy] 嗨"


def test_restore_sender_does_not_take_a_bracketed_message_for_the_sender():
    # 內容本身也以中括號開頭：模型丟了發送者時，[出售] 不能被當成發送者換掉
    assert restore_sender("[Amy] [WTS] gear", "[出售] 裝備") == "[Amy] [出售] 裝備"
    assert restore_sender("[Amy] [WTS] gear", "[艾米] [出售] 裝備") == "[Amy] [出售] 裝備"


def test_restore_sender_leaves_lines_without_a_sender_alone():
    assert restore_sender("hi", "嗨") == "嗨"


def test_translate_incoming_keeps_the_original_sender(capsys):
    fake = FakeHttpxClient(response=FakeResponse(content="[賈斯汀 渡鴉] 嗨"))
    assert _make(fake).translate_incoming("[贾斯廷 渡鸦] hi", []) == "[贾斯廷 渡鸦] 嗨"
    assert "model altered the sender prefix" in capsys.readouterr().err


def test_only_whitespace_after_the_sender_is_not_logged_as_an_altered_sender(capsys):
    fake = FakeHttpxClient(response=FakeResponse(content="[Amy]嗨"))
    assert _make(fake).translate_incoming("[Amy] hi", []) == "[Amy] 嗨"
    assert "model altered the sender prefix" not in capsys.readouterr().err


# --- has_stray_latin：譯文冒出原文沒有的英文（模型把名詞換成官方英文名）---
def test_has_stray_latin_flags_a_translation_that_switched_to_english():
    assert has_stray_latin("雪刺帽", "Snowspike Hat", "繁體中文（台灣）")


def test_has_stray_latin_flags_a_partly_englished_translation():
    # 實機：音譯的玩家名被還原成英文來源（卡拉米蒂 → Calamity），其餘照翻
    assert has_stray_latin("卡拉米蒂 现在等级 {0}！", "Calamity 現在等級 {0}！",
                              "繁體中文（台灣）")


def test_has_stray_latin_accepts_a_translation_in_the_target_language():
    assert not has_stray_latin("雪刺帽", "雪刺帽", "繁體中文（台灣）")


def test_has_stray_latin_allows_english_the_source_already_had():
    assert not has_stray_latin("death skeleturion", "死亡骷髏戰士 (Death Skeleturion)",
                                  "繁體中文（台灣）")


def test_has_stray_latin_is_off_for_a_latin_script_target_language():
    # 目標語言自己就以拉丁字母書寫時，譯文滿是拉丁字母才是對的
    assert not has_stray_latin("雪刺帽", "Snowspike Hat", "English")
    assert not has_stray_latin("雪刺帽", "Sombrero de Nieve", "Español")


def test_has_stray_latin_ignores_an_english_sender_name():
    assert not has_stray_latin("[Amy] 雪刺帽", "[Amy] 雪刺帽", "繁體中文（台灣）")


def test_has_stray_latin_looks_at_the_message_body_of_the_translation():
    assert has_stray_latin("[艾米] 雪刺帽", "[艾米] Snowspike Hat", "繁體中文（台灣）")


# --- 系統訊息落回英文時重譯一次 ---
def test_system_message_prompt_demands_the_whole_translation_in_the_target_language():
    assert "整則以 日本語 書寫" in build_system_message_system("日本語")


def test_strict_system_message_prompt_calls_out_the_english_slip():
    strict = build_system_message_system("日本語", strict=True)
    assert build_system_message_system("日本語") in strict
    assert "上一次" in strict


def test_translate_system_message_retries_when_the_model_answers_in_english():
    fake = _contents("Snowspike Hat", "雪刺帽")
    assert _make(fake).translate_system_message("雪刺帽") == "雪刺帽"
    assert len(fake.bodies) == 2
    assert "上一次" in fake.bodies[1]["messages"][0]["content"]   # 重譯用更嚴格的提示詞


def test_translate_system_message_does_not_retry_a_clean_translation():
    fake = _contents("雪刺帽")
    _make(fake).translate_system_message("雪刺帽")
    assert len(fake.bodies) == 1


def test_translate_system_message_keeps_a_retry_that_is_still_english():
    # 實測音譯的玩家名重譯仍會英譯：照樣顯示（呼叫端負責不快取），不再多打第三次
    fake = _contents("Calamity 現在等級 {0}！")
    tr = _make(fake)
    assert tr.translate_system_message("卡拉米蒂 现在等级 {0}！") == "Calamity 現在等級 {0}！"
    assert len(fake.bodies) == 2


def test_incoming_translation_is_not_retried():
    # 收訊有完整句子語境、實測不會落回英文；多打一次只是白花錢
    fake = _contents("Snowspike Hat")
    _make(fake).translate_incoming("雪刺帽", [])
    assert len(fake.bodies) == 1


def test_translator_exposes_the_target_language():
    assert _make(FakeHttpxClient()).target_language == "繁體中文（台灣）"


def test_reconfigure_updates_the_exposed_target_language():
    tr = _make(FakeHttpxClient())
    tr.reconfigure(provider="custom", base_url="http://x", model="m",
                   target_language="日本語")
    assert tr.target_language == "日本語"


# --- 規則 1：譯文整段都是拉丁，一個目標語言的字都沒有 ---
def test_has_stray_latin_flags_a_source_echoed_back_untranslated():
    # 拉丁文字的伺服器上主要的失敗樣態：模型把原文原樣吐回來。片段比對放行
    # （片段確實都在原文裡），要靠「整段沒有目標語言的字」這條才擋得住。
    assert has_stray_latin("You received Snowspike Hat",
                           "You received Snowspike Hat", "繁體中文（台灣）")


def test_has_stray_latin_accepts_a_translation_that_has_target_language_words():
    assert not has_stray_latin("You received Snowspike Hat",
                               "你獲得了 Snowspike Hat", "繁體中文（台灣）")


# --- 規則 2：譯文的拉丁片段不是原文本來就有的 ---
def test_has_stray_latin_flags_an_english_sentence_built_around_a_player_name():
    # 原文夾著英文玩家名，舊判準（只問原文有沒有英文）會整條放行
    assert has_stray_latin("收到 [Amy] 的伙伴邀请",
                           "Received a friend request from Amy", "繁體中文（台灣）")


def test_has_stray_latin_allows_a_player_name_copied_from_the_source():
    assert not has_stray_latin("收到 [Amy] 的伙伴邀请", "收到 Amy 的夥伴邀請",
                               "繁體中文（台灣）")


def test_has_stray_latin_ignores_case_and_spacing_when_matching_the_source():
    assert not has_stray_latin("你获得了 Lava Lily", "你獲得了  lava lily",
                               "繁體中文（台灣）")


def test_has_stray_latin_flags_a_name_the_model_swapped_in():
    # 原文有一個英文名，模型卻換上另一個 —— 片段不在原文裡就是憑空生成的
    assert has_stray_latin("收到 [Amy] 的伙伴邀请", "收到 Snowspike Hat 的夥伴邀請",
                           "繁體中文（台灣）")


def test_has_stray_latin_covers_non_latin_target_languages():
    for language in ["日本語", "한국어", "Русский", "ภาษาไทย", "Ελληνικά"]:
        assert has_stray_latin("雪刺帽", "Snowspike Hat", language)


def test_has_stray_latin_accepts_a_translation_without_any_latin():
    assert not has_stray_latin("你获得了 {0} 金币！", "{0} ゴールドを手に入れた！", "日本語")


def test_incoming_translation_logs_the_source_and_the_result(capsys):
    # app.log 要能把原文與實際譯文並排對照 —— messages.log 只留原文，不留譯文
    _make(FakeHttpxClient()).translate_incoming("[A] hi", ["[B] yo", "[A] sup"])
    err = capsys.readouterr().err
    assert "[translate] incoming done in" in err
    assert "model=m" in err and "ctx=2" in err
    assert "source='[A] hi'" in err and "translated='[A] 譯文'" in err


def test_outgoing_translation_logs_how_many_context_lines_it_carried(capsys):
    # 發話譯文被上下文帶偏時，第一個要看的就是當時餵了幾行
    _make(FakeHttpxClient()).translate_outgoing("哈囉", ["[B] yo"])
    err = capsys.readouterr().err
    assert "[translate] outgoing done in" in err and "ctx=1" in err


def test_system_message_retry_is_logged_apart_from_the_first_attempt(capsys):
    fake = FakeHttpxClient(response=FakeResponse(content="Lava Lily"))
    _make(fake).translate_system_message("你获得了 熔岩百合")
    err = capsys.readouterr().err
    assert "[translate] system message done in" in err
    assert "[translate] system message (strict retry) done in" in err


# --- 目標語言就是遊戲原生語言時，「不得改用英文」那幾句會自相矛盾 ---
def test_prompts_drop_the_no_english_clauses_when_the_target_is_the_game_language():
    rule = _game_noun_rule(OUTGOING_LANGUAGE)
    assert "不得改用英文" not in rule
    system = build_system_message_system(OUTGOING_LANGUAGE)
    assert "原文沒有的英文不得出現" not in system
    assert OUTGOING_LANGUAGE in system


def test_prompts_keep_the_no_english_clauses_for_other_latin_targets():
    # Español 仍要擋官方英文名：只有「目標＝遊戲語言」才拿掉，不是「拉丁字母就拿掉」
    assert "不得改用英文" in _game_noun_rule("Español")
    assert "原文沒有的英文不得出現" in build_system_message_system("Español")


def test_incoming_slang_rule_keeps_abbreviations_when_the_target_is_the_game_language():
    # 目標就是遊戲語言時「不要保留原縮寫」會逼模型把 lol 改寫掉
    assert "縮寫原樣保留" in build_incoming_system(OUTGOING_LANGUAGE)
    assert "縮寫原樣保留" not in build_incoming_system("日本語")


def test_game_language_match_ignores_case_and_spacing():
    assert "不得改用英文" not in _game_noun_rule(" english ")


def test_token_limit_rejected_under_both_names_raises_instead_of_looping():
    # 兩個名字互換是為了保住長度上限；但端點兩個都拒絕時不能無限互換
    fake = SequenceClient(_rejects("max_completion_tokens"),
                          _rejects("max_tokens", "unsupported"),
                          _rejects("max_completion_tokens"))
    t = Translator(provider="openai", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=fake)
    with pytest.raises(TranslatorConfigError):
        t.translate_incoming("[A] hi", [])
    assert len(fake.bodies) == 2


def test_sender_prefix_limit_is_shared_with_the_chat_parser():
    # 發送者前綴的長度上限由 markup 定義、postprocess 沿用：markup 收得下的名字，
    # has_stray_latin 也必須認得是前綴（否則拉丁字母的玩家名會被當成沒翻的內容）
    from src.reader.markup import lines_from_chatlog

    longest = "A" * 40
    raw = (f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
           f"<link;GID:1,{longest},2>[{longest}]</link> 你好 </color>")
    assert [line.text for line in lines_from_chatlog(raw)] == [f"[{longest}] 你好"]
    assert has_stray_latin(f"[{longest}] 你好", f"[{longest}] 哈囉", "繁體中文（台灣）") is False


def test_region_text_sends_the_recognized_lines_numbered():
    fake = FakeHttpxClient(response=FakeResponse(content="1. 跟莫爾談談\n2. 第二行"))
    translator = _make(fake)
    assert translator.translate_region_text("Talk to Merle Ambrose\nSecond line") == \
        "跟莫爾談談\n第二行"
    assert fake.last_body["messages"][-1] == {
        "role": "user", "content": "1. Talk to Merle Ambrose\n2. Second line"}
    assert fake.last_body["messages"][0]["content"] == build_region_system("繁體中文（台灣）")
    assert fake.last_body["max_tokens"] == _MAX_TOKENS_REGION


def test_region_text_fills_lines_the_model_dropped_with_the_original():
    fake = FakeHttpxClient(response=FakeResponse(content="2. 第二行"))
    assert _make(fake).translate_region_text("海报伙伴\nSecond line") == "海报伙伴\n第二行"


def test_region_text_strips_english_the_model_invented_for_a_line_without_any():
    # 實機：簡體中文原文「天国大本营」被譯成「天國大本營（Heavenly Headquarters）」，
    # 遊戲畫面上根本沒有這個英文名
    fake = FakeHttpxClient(response=FakeResponse(
        content="1. 若有時間，我希望你再次拜訪天國大本營（Heavenly Headquarters）！"))
    assert (_make(fake).translate_region_text("若有时间，我希望你再次拜访天国大本营！")
            == "若有時間，我希望你再次拜訪天國大本營！")


def test_region_text_keeps_english_copied_from_the_same_line_only():
    # 逐行判定：第一行原文有英文，括號照抄可留；第二行沒有，括號英文必是憑空生成
    fake = FakeHttpxClient(response=FakeResponse(
        content="1. 跟莫爾·安布羅斯（Merle Ambrose）談談\n2. 天國大本營（Heavenly HQ）"))
    assert (_make(fake).translate_region_text("Talk to Merle Ambrose\n天国大本营")
            == "跟莫爾·安布羅斯（Merle Ambrose）談談\n天國大本營")


def test_region_system_prompt_only_allows_parentheses_copied_from_the_line():
    prompt = build_region_system("繁體中文（台灣）")
    assert "一律用半形括號附上原文" not in prompt   # 舊規則：沒有英文可抄時模型會自己翻一個
    assert "逐字照抄" in prompt


def test_number_lines_skips_blank_lines():
    assert number_lines("a\n\n b \n") == (["a", "b"], "1. a\n2. b")


def test_unnumber_lines_accepts_various_number_styles_and_plain_output():
    assert unnumber_lines("1) 甲\n２．乙\n3、丙", ["a", "b", "c"]) == "甲\n乙\n丙"
    assert unnumber_lines("甲\n乙", ["a", "b"]) == "甲\n乙"        # 沒編號但行數相同
    assert unnumber_lines("一整段", ["a", "b"]) == "一整段"        # 對不上就原樣回傳


def test_translate_region_text_logs_no_translation_content(monkeypatch):
    # 區域翻譯的原文可能整頁、譯文可能很長 —— log 只留字數，不留內容
    import src.translation.translator as translator_module

    messages = []
    monkeypatch.setattr(translator_module, "log", messages.append)
    fake = FakeHttpxClient()
    text = "Talk to Merle Ambrose"
    translated = _make(fake).translate_region_text(text)
    assert translated == "譯文"
    assert not any("譯文" in m or "Merle" in m for m in messages)
    assert any(f"translated=<{len(translated)} chars>" in m and
              f"source='<text {len(text)} chars, 1 lines>'" in m for m in messages)


def test_region_system_prompt_carries_the_target_language_and_no_chat_format():
    prompt = build_region_system("日本語")
    assert "日本語" in prompt
    assert "[發送者]" not in prompt


def test_region_system_prompt_speaks_of_recognized_text_not_of_an_image():
    # 輸入固定是本機 OCR 的文字，看圖時代的句子（描述畫面、猜字、沒有文字）只是噪音
    prompt = build_region_system("日本語")
    for stale in ("圖片", "描述畫面", "沒有文字", "無法辨識"):
        assert stale not in prompt


# --- 串流與取消：請求進行中可以從別的執行緒斷線，伺服器停止生成、不再計費 ---
def test_openai_compat_asks_for_a_stream_and_joins_the_chunks():
    fake = FakeHttpxClient(response=FakeResponse(content="一段很長的譯文"))
    assert _make(fake).translate_outgoing("哈囉", []) == "一段很長的譯文"
    assert fake.last_body["stream"] is True


def test_only_the_official_openai_endpoint_asks_for_usage_in_the_stream():
    fake = FakeHttpxClient()
    Translator(provider="openai", model="gpt-4o-mini", api_key="k",
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.last_body["stream_options"] == {"include_usage": True}
    fake = FakeHttpxClient()
    _make(fake).translate_incoming("[A] hi", [])
    assert "stream_options" not in fake.last_body   # 自架後端不一定認得，別冒 400 的險


def test_openai_compat_truncation_is_read_from_the_stream():
    fake = FakeHttpxClient(response=FakeResponse(content="呃 呃", finish_reason="length",
                                                 completion_tokens=3))
    with pytest.raises(TranslatorBadOutput) as ei:
        _make(fake).translate_incoming("[A] hi", [])
    assert "completion_tokens=3" in str(ei.value)


class BlockingResponse(FakeResponse):
    """送出第一塊後卡住，直到被 close 才以連線錯誤結束：模擬模型還在生成時客戶端斷線。"""

    def __init__(self):
        super().__init__()
        self.started = threading.Event()
        self.released = threading.Event()

    def iter_lines(self):
        yield _sse([{"choices": [{"delta": {"content": "譯"}, "finish_reason": None}]}])[0]
        self.started.set()
        self.released.wait(5)
        raise httpx.ReadError("connection closed")

    def close(self):
        super().close()
        self.released.set()


def _run_in_thread(fn):
    outcome = {}

    def target():
        try:
            outcome["result"] = fn()
        except Exception as exc:
            outcome["error"] = exc

    thread = threading.Thread(target=target, daemon=True)
    thread.start()
    return thread, outcome


def test_cancelling_an_openai_compat_request_closes_the_stream():
    resp = BlockingResponse()
    fake = FakeHttpxClient(response=resp)
    handle = RequestHandle()
    thread, outcome = _run_in_thread(
        lambda: _make(fake).translate_region_text("Hello", cancel=handle))
    assert resp.started.wait(5)
    handle.cancel()
    thread.join(5)
    assert resp.closed and handle.cancelled
    assert isinstance(outcome.get("error"), TranslatorCancelled)


def test_a_request_cancelled_before_it_starts_is_never_sent():
    handle = RequestHandle()
    handle.cancel()
    fake = FakeHttpxClient()
    with pytest.raises(TranslatorCancelled):
        _make(fake).translate_outgoing("哈囉", [], cancel=handle)
    assert fake.last_body is None


def test_a_connection_error_without_a_cancel_is_still_offline():
    resp = BlockingResponse()
    resp.released.set()   # 沒人取消、連線自己斷了
    fake = FakeHttpxClient(response=resp)
    with pytest.raises(TranslatorOffline):
        _make(fake).translate_outgoing("哈囉", [], cancel=RequestHandle())


def test_the_request_handle_is_released_after_the_request_completes():
    fake = FakeHttpxClient()
    handle = RequestHandle()
    _make(fake).translate_outgoing("哈囉", [], cancel=handle)
    handle.cancel()   # 事後取消不該去關一個已經結束的回應
    assert not fake._response.closed


class BlockingMessageStream(FakeMessageStream):
    def __init__(self):
        super().__init__("end_turn", "克勞德譯文")
        self.started = threading.Event()
        self.released = threading.Event()

    def get_final_message(self):
        self.started.set()
        self.released.wait(5)
        raise httpx2.ReadError("connection closed")

    def close(self):
        super().close()
        self.released.set()


class BlockingAnthropicMessages(FakeAnthropicMessages):
    @contextmanager
    def stream(self, **kwargs):
        self.last_kwargs = kwargs
        self.last_stream = BlockingMessageStream()
        yield self.last_stream


def test_claude_uses_the_streaming_helper():
    fake = FakeAnthropicClient()
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=fake)
    assert t.translate_incoming("[A] hi", []) == "[A] 克勞德譯文"
    assert fake.messages.last_stream is not None


def test_cancelling_a_claude_request_closes_the_stream():
    fake = FakeAnthropicClient()
    fake.messages = BlockingAnthropicMessages()
    handle = RequestHandle()
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=fake)
    thread, outcome = _run_in_thread(lambda: t.translate_region_text("Hello", cancel=handle))
    assert fake.messages.last_stream.started.wait(5)
    handle.cancel()
    thread.join(5)
    assert fake.messages.last_stream.closed
    assert isinstance(outcome.get("error"), TranslatorCancelled)


def test_region_system_prompt_pins_the_order_of_name_and_parenthesized_original():
    # 實機：模型寫成「CrownShop（皇冠商店）」，譯名與括號原文對調了
    prompt = build_region_system("繁體中文（台灣）")
    assert "譯名（原文）" in prompt and "不可對調" in prompt


def test_prompts_ask_to_translate_parentheses_already_in_the_source():
    # 括號被定義成「附原文用」後，模型會把玩家自己打的 (rank 5) 也當成原文照抄或刪掉
    for build in (build_incoming_system, build_system_message_system, build_region_system):
        assert "原文本來就有的括號" in build("日本語")


def test_game_noun_rule_treats_joined_camel_case_names_as_nouns_not_code():
    # 實機：`CrownShop`（連寫）被當成代碼保留、譯名進了括號；`Crown Shop` 就正常
    rule = _game_noun_rule("繁體中文（台灣）")
    assert "連寫" in rule and "不是代碼" in rule


def test_game_noun_rule_pins_the_order_of_name_and_original_for_every_prompt():
    # 框選實機撞到「CrownShop（皇冠商店）」；慣例只有一份，聊天翻譯要一起講死
    rule = _game_noun_rule("繁體中文（台灣）")
    assert "譯名（原文）" in rule and "不可對調" in rule
    assert "不可對調" in build_incoming_system("繁體中文（台灣）")
    assert "不可對調" in build_system_message_system("繁體中文（台灣）")


JA = "\n".join([
    "1. [Amy] 知らないよ、Kai と新しい鎧を手に入れて、"
    "巨像大道（Colossus Boulevard）で火猫（Fire Cat）を覚えた、笑",
    "2. Kai が火猫（Fire Cat）を教えてくれた！巨像大道（Colossus Boulevard）で {0} ゴールドを獲得した。",
    "3. 火猫（Fire Cat）に話しかける",
    "4. 巨像大道（Colossus Boulevard）へ行く",
    "5. そしてあなたは",
])


def test_translate_incoming_sends_the_example_before_the_context():
    fake = FakeHttpxClient()
    tr = _make(fake)
    fp = tr.examples_fingerprint
    assert tr.set_examples(ExampleSet(("src", "out"), None, None), fp)
    tr.translate_incoming("[A] hi", ["[B] yo"])
    turns = _turns(fake.last_body)
    assert turns[:2] == [{"role": "user", "content": "src"}, {"role": "assistant", "content": "out"}]
    assert turns[-1] == {"role": "user", "content": "[A] hi"}


def test_without_examples_no_example_turns_are_sent():
    fake = FakeHttpxClient()
    _make(fake).translate_system_message("你获得了 {0} 金币！")
    assert _turns(fake.last_body) == [{"role": "user", "content": "你获得了 {0} 金币！"}]


def test_region_text_sends_the_region_example_first():
    fake = FakeHttpxClient(response=FakeResponse(content="1. 譯文"))
    tr = _make(fake)
    tr.set_examples(ExampleSet(None, None, ("1. a", "1. b")), tr.examples_fingerprint)
    tr.translate_region_text("hello")
    assert _turns(fake.last_body)[:2] == [{"role": "user", "content": "1. a"},
                                          {"role": "assistant", "content": "1. b"}]


def test_set_examples_ignores_a_stale_fingerprint():
    tr = _make(FakeHttpxClient())
    assert not tr.set_examples(ExampleSet(("s", "o"), None, None), "other")
    assert tr.examples is None


def test_reconfigure_clears_the_examples():
    tr = _make(FakeHttpxClient())
    tr.set_examples(ExampleSet(("s", "o"), None, None), tr.examples_fingerprint)
    tr.reconfigure(provider="custom", base_url="http://x", model="m", target_language="日本語")
    assert tr.examples is None


def test_reconfigure_without_changes_keeps_the_examples():
    tr = _make(FakeHttpxClient())
    tr.set_examples(ExampleSet(("s", "o"), None, None), tr.examples_fingerprint)
    assert not tr.reconfigure(provider="custom", base_url="http://x", model="m",
                              target_language="繁體中文（台灣）")
    assert tr.examples is not None


def test_generate_examples_retries_bad_output_and_merges():
    good_but_no_system = JA.replace("{0} ゴールド", "ゴールド")
    good_but_no_incoming = JA.replace("[Amy] ", "")
    fake = _contents("garbage", good_but_no_system, good_but_no_incoming)
    tr = Translator(target_language="日本語", client=fake, provider="custom", base_url="http://x", model="m")
    fp, examples = tr.generate_examples()
    assert examples.complete and len(fake.bodies) == 3 and fp == tr.examples_fingerprint
    assert all("temperature" not in body for body in fake.bodies)


def test_generate_examples_gives_up_after_six_requests():
    fake = _contents(*["garbage"] * 7)
    tr = Translator(target_language="日本語", client=fake, provider="custom", base_url="http://x", model="m")
    _, examples = tr.generate_examples()
    assert examples == EMPTY and len(fake.bodies) == 6


def test_generate_examples_does_not_retry_a_config_error():
    fake = FakeHttpxClient(response=FakeResponse(status_code=401, text='{"error":{"message":"bad key"}}'))
    tr = Translator(target_language="日本語", client=fake, provider="custom", base_url="http://x", model="m")
    with pytest.raises(TranslatorConfigError):
        tr.generate_examples()


def test_generate_examples_for_the_game_language_sends_nothing():
    fake = FakeHttpxClient()
    tr = Translator(target_language="English", client=fake, provider="custom", base_url="http://x", model="m")
    assert tr.generate_examples()[1] == GAME_LANGUAGE_EXAMPLES and fake.last_body is None


def test_generate_and_store_reuses_a_cached_set_without_a_request(tmp_path, monkeypatch):
    store = ExampleStore(tmp_path / "ex.json")
    api = {"provider": "custom", "base_url": "http://x", "model": "m"}
    cached = ExampleSet(("s", "o"), None, None)
    store.put(examples_fingerprint(api, "日本語"), cached)
    monkeypatch.setattr(Translator, "generate_examples", lambda self: pytest.fail("no request expected"))
    assert generate_and_store(api, "日本語", store) == cached


def test_test_translate_survives_a_failing_example_generation(tmp_path, monkeypatch):
    monkeypatch.setattr(Translator, "translate_incoming", lambda self, text, ctx: "譯文")
    monkeypatch.setattr(Translator, "generate_examples",
                        lambda self: (_ for _ in ()).throw(TranslatorOffline("down")))
    api = {"provider": "custom", "base_url": "http://x", "model": "m"}
    assert run_test_translate(api, "日本語", example_store=ExampleStore(tmp_path / "ex.json")) == "譯文"


def test_strict_retry_sends_the_same_example_turns():
    fake = _contents("Gained gold", "Gained gold")
    tr = _make(fake)
    tr.set_examples(ExampleSet(None, ("s", "o"), None), tr.examples_fingerprint)
    tr.translate_system_message("Gained gold")
    assert len(fake.bodies) == 2
    assert _turns(fake.bodies[0])[:2] == _turns(fake.bodies[1])[:2] == [
        {"role": "user", "content": "s"}, {"role": "assistant", "content": "o"}]


def test_generate_and_store_persists_a_non_empty_result(tmp_path, monkeypatch):
    store = ExampleStore(tmp_path / "ex.json")
    api = {"provider": "custom", "base_url": "http://x", "model": "m"}
    made = ExampleSet(("s", "o"), None, None)
    monkeypatch.setattr(Translator, "generate_examples", lambda self: ("fp", made))
    assert generate_and_store(api, "日本語", store) == made
    assert store.get(examples_fingerprint(api, "日本語")) == made


def test_generate_and_store_skips_writing_an_empty_result(tmp_path, monkeypatch):
    store = ExampleStore(tmp_path / "ex.json")
    api = {"provider": "custom", "base_url": "http://x", "model": "m"}
    monkeypatch.setattr(Translator, "generate_examples", lambda self: ("fp", EMPTY))
    assert generate_and_store(api, "日本語", store) is None
    assert store.get(examples_fingerprint(api, "日本語")) is None


class ReconfiguringSequenceClient(SequenceClient):
    """第一個請求送出後設定就被換成另一個目標語言；`translator` 由測試接上。"""

    translator = None

    @contextmanager
    def stream(self, method, url, json):
        with super().stream(method, url, json) as response:
            if len(self.bodies) == 1:
                self.translator.reconfigure(provider="custom", base_url="http://x",
                                            model="m2", target_language="한국어")
            yield response


def _late_client(monkeypatch) -> SequenceClient:
    """之後 reconfigure 重建的後端都接到回傳的假 client，用來看請求有沒有跑去新設定。"""
    import src.translation.translator as module
    late = SequenceClient(FakeResponse())
    build = module._build_client
    monkeypatch.setattr(module, "_build_client", lambda **api: build(**api, client=late))
    return late


def test_a_reconfigure_mid_request_does_not_mix_settings_in_the_strict_retry(monkeypatch):
    fake = ReconfiguringSequenceClient(FakeResponse(content="Gained gold"))
    fake.translator = tr = _make(fake)
    late = _late_client(monkeypatch)
    tr.set_examples(ExampleSet(None, ("s", "o"), None), tr.examples_fingerprint)
    tr.translate_system_message("Gained gold")
    assert late.bodies == [] and len(fake.bodies) == 2
    for body in fake.bodies:
        assert body["messages"][0]["content"] == build_system_message_system(
            "繁體中文（台灣）", strict=body is fake.bodies[1])
        assert _turns(body)[:2] == [{"role": "user", "content": "s"},
                                    {"role": "assistant", "content": "o"}]


@pytest.mark.parametrize("builder, call", [
    ("build_incoming_system", lambda tr: tr.translate_incoming("[A] hi", [])),
    ("build_region_system", lambda tr: tr.translate_region_text("hello")),
])
def test_a_reconfigure_after_the_prompt_is_built_still_uses_the_same_backend(
        monkeypatch, builder, call):
    import src.translation.translator as module
    fake = SequenceClient(FakeResponse(content="1. 譯文"))
    tr = _make(fake)
    late = _late_client(monkeypatch)
    build = getattr(module, builder)

    def build_then_reconfigure(target_language):
        tr.reconfigure(provider="custom", base_url="http://x", model="m2",
                       target_language="한국어")
        return build(target_language)

    monkeypatch.setattr(module, builder, build_then_reconfigure)
    call(tr)
    assert late.bodies == [] and len(fake.bodies) == 1
    assert fake.bodies[0]["messages"][0]["content"] == build("繁體中文（台灣）")


def test_generate_examples_returns_the_fingerprint_of_the_settings_it_used(monkeypatch):
    fake = ReconfiguringSequenceClient(FakeResponse(content="garbage"), FakeResponse(content=JA))
    fake.translator = tr = Translator(target_language="日本語", client=fake,
                                      provider="custom", base_url="http://x", model="m")
    late = _late_client(monkeypatch)
    api = {"provider": "custom", "base_url": "http://x", "model": "m"}
    fp, examples = tr.generate_examples()
    assert fp == examples_fingerprint(api, "日本語") != tr.examples_fingerprint
    assert examples.complete and late.bodies == [] and len(fake.bodies) == 2


def test_a_request_on_a_closed_client_is_offline_so_the_pool_retries_it():
    # 請求開頭讀到的設定剛被 reconfigure 換掉時舊 client 已關，httpx 會拋 RuntimeError
    tr = Translator(provider="custom", base_url="http://x", model="m",
                    target_language="繁體中文（台灣）")
    tr.close()
    with pytest.raises(TranslatorOffline):
        tr.translate_incoming("[A] hi", [])


def test_a_request_on_a_closed_claude_client_is_offline():
    tr = Translator(provider="claude", model="m", api_key="k", target_language="繁體中文（台灣）",
                    client=anthropic.Anthropic(api_key="k", max_retries=0))
    tr.close()
    with pytest.raises(TranslatorOffline):
        tr.translate_incoming("[A] hi", [])


def test_an_unrelated_runtime_error_is_not_taken_for_offline():
    tr = _make(FakeHttpxClient(raises=RuntimeError("boom")))
    with pytest.raises(RuntimeError, match="boom") as raised:
        tr.translate_incoming("[A] hi", [])
    assert not isinstance(raised.value, TranslatorOffline)
