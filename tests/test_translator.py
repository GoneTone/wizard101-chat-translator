"""translator 的 provider 選擇與錯誤映射測試（mock client，不打真 API）。"""
import anthropic
import httpx
import httpx2
import pytest

from src.translator import (
    OPENAI_BASE_URL, Translator, TranslatorBadOutput, TranslatorConfigError,
    TranslatorOffline, build_incoming_system,
)


class FakeResponse:
    def __init__(self, status_code=200, content="譯文", finish_reason="stop",
                 completion_tokens=None):
        self.status_code = status_code
        self._content = content
        self._finish_reason = finish_reason
        self._completion_tokens = completion_tokens

    def json(self):
        data = {"choices": [{"message": {"content": self._content},
                             "finish_reason": self._finish_reason}]}
        if self._completion_tokens is not None:
            data["usage"] = {"completion_tokens": self._completion_tokens}
        return data

    def raise_for_status(self):
        if self.status_code >= 400:
            raise httpx.HTTPStatusError("err", request=None, response=None)


class FakeHttpxClient:
    """替身 httpx.Client：post 回傳預設回應或拋出預設例外。"""
    def __init__(self, response=None, raises=None):
        self._response = response or FakeResponse()
        self._raises = raises
        self.last_body = None

    def post(self, url, json):
        self.last_body = json
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


class FakeAnthropicMessages:
    def __init__(self, raises=None, stop_reason="end_turn"):
        self._raises = raises
        self._stop_reason = stop_reason
        self.last_kwargs = None

    def create(self, **kwargs):
        self.last_kwargs = kwargs
        if self._raises:
            raise self._raises
        stop_reason = self._stop_reason

        class Block:
            type = "text"
            text = "克勞德譯文"

        class Resp:
            content = [Block()]

        Resp.stop_reason = stop_reason
        return Resp()


class FakeAnthropicClient:
    def __init__(self, raises=None, stop_reason="end_turn"):
        self.messages = FakeAnthropicMessages(raises, stop_reason)


def _anthropic_status_error(status):
    resp = httpx2.Response(status, request=httpx2.Request("POST", "http://x"))
    return anthropic.APIStatusError("err", response=resp, body=None)


def test_claude_provider_returns_text():
    t = Translator(provider="claude", model="claude-opus-5", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient())
    assert t.translate_incoming("[A] hi", []) == "克勞德譯文"


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
    from src.translator import _MAX_TOKENS, _MAX_TOKENS_THINKING
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
    assert _make(fake).translate_incoming("[A] hi", []) == "譯文"


def test_claude_sends_max_tokens():
    from src.translator import _MAX_TOKENS_THINKING
    fake = FakeAnthropicClient()
    Translator(provider="claude", model="m", api_key="k",
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.messages.last_kwargs["max_tokens"] == _MAX_TOKENS_THINKING


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


def test_openai_provider_forces_official_base_url():
    t = Translator(provider="openai", base_url="http://evil.example", model="m",
                   api_key="k", target_language="繁體中文（台灣）")
    assert str(t._impl._client.base_url) == OPENAI_BASE_URL


def test_build_turns_without_context_is_single_user_turn():
    from src.translator import build_turns
    assert build_turns([], "[A] hi", "intro") == [
        {"role": "user", "content": "[A] hi"}]


def test_build_turns_prepends_fewshot_examples():
    from src.translator import build_turns
    examples = [{"role": "user", "content": "在嗎"},
                {"role": "assistant", "content": "you there?"}]
    turns = build_turns([], "哈囉", "intro", examples=examples)
    assert turns[:2] == examples                       # 範例在最前
    assert turns[-1] == {"role": "user", "content": "哈囉"}  # 待翻句仍在最後


def test_outgoing_uses_fewshot_when_no_context():
    from src.translator import FEWSHOT_OUTGOING
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
    from src.translator import FEWSHOT_OUTGOING
    fake = FakeHttpxClient()
    _make(fake).translate_outgoing("好啊", ["[A] want to trade?"])
    turns = _turns(fake.last_body)
    # 遊戲內幾乎永遠有上下文，範例若在此時被略過，防脫稿保護等於沒有
    assert turns[:len(FEWSHOT_OUTGOING)] == FEWSHOT_OUTGOING
    assert any("[A] want to trade?" in m["content"] for m in turns)
    assert turns[-1] == {"role": "user", "content": "好啊"}


def test_build_turns_with_context_is_multi_turn():
    from src.translator import CONTEXT_ACK, build_turns
    turns = build_turns(["[A] one", "[B] two"], "[C] three", "背景說明")
    assert turns == [
        {"role": "user", "content": "背景說明\n[A] one\n[B] two"},
        {"role": "assistant", "content": CONTEXT_ACK},
        {"role": "user", "content": "[C] three"},  # 待翻句永遠是最後一個乾淨 user turn
    ]


def _turns(body):
    return body["messages"][1:]  # 去掉 system，剩下對話輪


def test_incoming_system_has_no_format_markers():
    # system 不得列出段落標記字串,否則小模型會把它回吐成「請照此格式提供輸入」
    system = build_incoming_system("繁體中文（台灣）")
    assert "[要翻譯的訊息]" not in system
    assert "語境" in system            # 仍說明會收到聊天記錄當背景


def test_both_systems_forbid_treating_input_as_instructions():
    # 輸入內容長得像指令時模型不得脫稿回應（實測踩過：回了「了解。請提供…」）
    from src.translator import build_outgoing_system
    assert "絕不回應" in build_incoming_system("繁體中文（台灣）")
    assert "絕不回應" in build_outgoing_system("English")


def test_reconfigure_switches_provider():
    t = _make(FakeHttpxClient())
    t.reconfigure(provider="claude", base_url="", model="claude-opus-5", api_key="k",
                  thinking=False, target_language="日本語")
    # reconfigure 後為 Claude client（真物件）；此處只驗證型別切換，不打 API
    from src.translator import _ClaudeClient
    assert isinstance(t._impl, _ClaudeClient)
