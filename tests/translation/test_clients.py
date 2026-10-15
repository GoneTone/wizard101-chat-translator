"""clients 的 HTTP 協定測試：請求內容、錯誤碼對應、SSE 串流、取消與 token 上限（mock client，不打真 API）。
兩種後端都走串流：假 client 回 SSE 行（OpenAI 相容）或假的 MessageStream（Claude），
取消測試用「卡住直到被 close」的回應模擬模型還在生成時客戶端主動斷線。"""
import json
import threading
from contextlib import contextmanager
from types import SimpleNamespace

import anthropic
import httpx
import httpx2
import pytest

from src.translation.clients import (
    _CLAUDE_OUTPUT_FORMAT,
    MAX_TOKENS_THINKING,
    RequestHandle,
    error_detail,
)
from src.translation.errors import (
    TranslatorBadOutput,
    TranslatorCancelled,
    TranslatorConfigError,
    TranslatorNoModelList,
    TranslatorOffline,
)
from src.translation.translator import Translator, list_models
from tests.translation.translator_fakes import FakeHttpxClient, FakeResponse, SequenceClient, _make, _sse


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
    """替身 MessageStream：`get_final_message` 回累積好的訊息。
    `content` 是譯文，包成結構化輸出的 JSON；`blocks` 給定時改回這串原樣 (type, text)。"""

    def __init__(self, stop_reason, content, blocks=None):
        self._stop_reason = stop_reason
        self._blocks = blocks or [("text", json.dumps({"translation": content}))]
        self.closed = False

    def get_final_message(self):
        class Resp:
            content = [SimpleNamespace(type=kind, text=text) for kind, text in self._blocks]
            stop_reason = self._stop_reason

        return Resp()

    def close(self):
        self.closed = True


class FakeAnthropicMessages:
    def __init__(self, raises=None, stop_reason="end_turn", content="克勞德譯文", blocks=None):
        self._raises = raises
        self._stop_reason = stop_reason
        self._content = content
        self._blocks = blocks
        self.last_kwargs = None
        self.last_stream = None

    @contextmanager
    def stream(self, **kwargs):
        self.last_kwargs = kwargs
        if self._raises:
            raise self._raises
        self.last_stream = FakeMessageStream(self._stop_reason, self._content, self._blocks)
        yield self.last_stream


class FakeAnthropicClient:
    def __init__(self, raises=None, stop_reason="end_turn", content="克勞德譯文", blocks=None):
        self.messages = FakeAnthropicMessages(raises, stop_reason, content, blocks)


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
    from src.translation.clients import MAX_TOKENS
    off = FakeHttpxClient()
    Translator(provider="custom", base_url="http://x", model="m", thinking=False,
               target_language="繁體中文（台灣）", client=off).translate_incoming("[A] hi", [])
    assert off.last_body["max_tokens"] == MAX_TOKENS
    on = FakeHttpxClient()
    Translator(provider="custom", base_url="http://x", model="m", thinking=True,
               target_language="繁體中文（台灣）", client=on).translate_incoming("[A] hi", [])
    assert on.last_body["max_tokens"] == MAX_TOKENS_THINKING


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
    assert fake.messages.last_kwargs["max_tokens"] == MAX_TOKENS_THINKING


def test_claude_auto_effort_sends_no_effort():
    # 自動＝維持模型預設（adaptive），不帶 effort；輸出格式照樣限定
    fake = FakeAnthropicClient()
    Translator(provider="claude", model="m", api_key="k",
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.messages.last_kwargs["output_config"] == {"format": _CLAUDE_OUTPUT_FORMAT}


def test_claude_low_effort_sends_output_config():
    from src.config.services import EFFORT_LOW
    fake = FakeAnthropicClient()
    Translator(provider="claude", model="m", api_key="k", effort=EFFORT_LOW,
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.messages.last_kwargs["output_config"] == {
        "format": _CLAUDE_OUTPUT_FORMAT, "effort": "low"}
    # 思考深度壓低不代表關閉思考：Claude 沒有 thinking 開關可送
    assert "thinking" not in fake.messages.last_kwargs


def test_claude_truncated_output_maps_to_bad_output():
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）",
                   client=FakeAnthropicClient(stop_reason="max_tokens"))
    with pytest.raises(TranslatorBadOutput):
        t.translate_incoming("[A] hi", [])


def test_claude_reads_the_first_text_block():
    blocks = [("text", json.dumps({"translation": "[A] 第一版"})), ("thinking", ""),
              ("text", json.dumps({"translation": "[A] 第二版"}))]
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient(blocks=blocks))
    assert t.translate_incoming("[A] hi", []) == "[A] 第一版"


@pytest.mark.parametrize("blocks", [[("text", "")], [("text", "[A] 不是 JSON")],
                                    [("text", '{"other": "x"}')], [("thinking", "")]])
def test_claude_unexpected_output_maps_to_bad_output(blocks):
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient(blocks=blocks))
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
    from src.translation.clients import MAX_TOKENS

    fake = FakeHttpxClient()
    t = Translator(provider="openai", model="m", api_key="k", thinking=False,
                   target_language="繁體中文（台灣）", client=fake)
    t.translate_incoming("[A] hi", [])
    assert fake.last_body["max_completion_tokens"] == MAX_TOKENS
    assert "max_tokens" not in fake.last_body
    assert "temperature" not in fake.last_body
    fake = FakeHttpxClient()
    Translator(provider="openai", model="m", api_key="k", thinking=True,
               target_language="繁體中文（台灣）", client=fake).translate_incoming("[A] hi", [])
    assert fake.last_body["max_completion_tokens"] == MAX_TOKENS_THINKING


def test_custom_endpoint_keeps_max_tokens_and_temperature():
    # 自架後端（vLLM／Ollama／LM Studio）多半只認 max_tokens，temperature=0 也是為了它們的重現性
    fake = FakeHttpxClient()
    _make(fake).translate_incoming("[A] hi", [])
    assert "max_tokens" in fake.last_body
    assert "max_completion_tokens" not in fake.last_body
    assert fake.last_body["temperature"] == 0


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
    assert fake.bodies[1]["max_tokens"] == MAX_TOKENS_THINKING
    fake = SequenceClient(_rejects("max_tokens", "unsupported"), FakeResponse())
    _make(fake).translate_incoming("[A] hi", [])
    assert "max_tokens" not in fake.bodies[1]
    assert fake.bodies[1]["max_completion_tokens"] == MAX_TOKENS_THINKING


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
