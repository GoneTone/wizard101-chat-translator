"""translator 的 provider 選擇與錯誤映射測試（mock client，不打真 API）。"""
import anthropic
import httpx
import httpx2
import pytest

from src.translator import (
    OPENAI_BASE_URL, Translator, TranslatorConfigError, TranslatorOffline,
    build_incoming_system,
)


class FakeResponse:
    def __init__(self, status_code=200, content="譯文"):
        self.status_code = status_code
        self._content = content

    def json(self):
        return {"choices": [{"message": {"content": self._content}}]}

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
    assert _make(fake).translate_outgoing("哈囉") == "譯文"
    assert fake.last_body["temperature"] == 0


def test_openai_compat_connection_error_maps_to_offline():
    fake = FakeHttpxClient(raises=httpx.ConnectError("refused"))
    with pytest.raises(TranslatorOffline):
        _make(fake).translate_incoming("[A] hi")


@pytest.mark.parametrize("status", [401, 403, 404])
def test_openai_compat_auth_or_model_error_maps_to_config_error(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorConfigError) as ei:
        _make(fake).translate_incoming("[A] hi")
    assert ei.value.status == status


@pytest.mark.parametrize("status", [429, 500, 503])
def test_openai_compat_retryable_status_maps_to_offline(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorOffline):
        _make(fake).translate_incoming("[A] hi")


class FakeAnthropicMessages:
    def __init__(self, raises=None):
        self._raises = raises

    def create(self, **kwargs):
        if self._raises:
            raise self._raises

        class Block:
            type = "text"
            text = "克勞德譯文"

        class Resp:
            content = [Block()]

        return Resp()


class FakeAnthropicClient:
    def __init__(self, raises=None):
        self.messages = FakeAnthropicMessages(raises)


def _anthropic_status_error(status):
    resp = httpx2.Response(status, request=httpx2.Request("POST", "http://x"))
    return anthropic.APIStatusError("err", response=resp, body=None)


def test_claude_provider_returns_text():
    t = Translator(provider="claude", model="claude-opus-5", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient())
    assert t.translate_incoming("[A] hi") == "克勞德譯文"


def test_claude_connection_error_maps_to_offline():
    err = anthropic.APIConnectionError(request=httpx2.Request("POST", "http://x"))
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient(raises=err))
    with pytest.raises(TranslatorOffline):
        t.translate_incoming("[A] hi")


@pytest.mark.parametrize("status,exc", [(401, TranslatorConfigError),
                                        (404, TranslatorConfigError),
                                        (429, TranslatorOffline),
                                        (500, TranslatorOffline)])
def test_claude_status_error_mapping(status, exc):
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）",
                   client=FakeAnthropicClient(raises=_anthropic_status_error(status)))
    with pytest.raises(exc):
        t.translate_incoming("[A] hi")


def test_openai_provider_disables_thinking_with_official_param_only():
    # openai provider 關閉思考時只帶官方認得的 reasoning_effort，
    # 不得夾帶自架後端專用參數（官方端點對未知欄位嚴格回 400）。
    fake = FakeHttpxClient()
    t = Translator(provider="openai", model="m", api_key="k", thinking=False,
                   target_language="繁體中文（台灣）", client=fake)
    t.translate_incoming("[A] hi")
    assert fake.last_body["reasoning_effort"] == "none"
    for key in ("chat_template_kwargs", "think", "enable_thinking"):
        assert key not in fake.last_body


def test_openai_provider_thinking_on_sends_no_thinking_params():
    fake = FakeHttpxClient()
    t = Translator(provider="openai", model="m", api_key="k", thinking=True,
                   target_language="繁體中文（台灣）", client=fake)
    t.translate_incoming("[A] hi")
    for key in ("reasoning_effort", "chat_template_kwargs", "think", "enable_thinking"):
        assert key not in fake.last_body


def test_openai_provider_forces_official_base_url():
    t = Translator(provider="openai", base_url="http://evil.example", model="m",
                   api_key="k", target_language="繁體中文（台灣）")
    assert str(t._impl._client.base_url) == OPENAI_BASE_URL


def test_compose_user_message_without_context_is_plain_text():
    from src.translator import INCOMING_TARGET_HEADER, compose_user_message
    assert compose_user_message([], "[A] hi", INCOMING_TARGET_HEADER) == "[A] hi"


def test_compose_user_message_with_context_has_sections():
    from src.translator import (
        CONTEXT_HEADER, INCOMING_TARGET_HEADER, compose_user_message,
    )
    msg = compose_user_message(["[A] one", "[B] two"], "[C] three",
                               INCOMING_TARGET_HEADER)
    assert msg == (f"{CONTEXT_HEADER}\n[A] one\n[B] two\n\n"
                   f"{INCOMING_TARGET_HEADER}\n[C] three")


def test_incoming_history_feeds_next_translation():
    from src.translator import CONTEXT_HEADER
    fake = FakeHttpxClient()
    t = _make(fake)
    t.translate_incoming("[A] one")
    assert CONTEXT_HEADER not in fake.last_body["messages"][1]["content"]  # 首行無上下文
    t.translate_incoming("[B] two")
    user = fake.last_body["messages"][1]["content"]
    assert CONTEXT_HEADER in user
    assert "[A] one" in user          # 上一行成為上下文
    assert user.endswith("[B] two")


def test_failed_translation_not_recorded_to_history():
    from src.translator import CONTEXT_HEADER

    class FailOnceClient(FakeHttpxClient):
        def __init__(self):
            super().__init__()
            self.calls = 0

        def post(self, url, json):
            self.calls += 1
            self.last_body = json
            if self.calls == 1:
                raise httpx.ConnectError("refused")
            return self._response

    fake = FailOnceClient()
    t = _make(fake)
    with pytest.raises(TranslatorOffline):
        t.translate_incoming("[A] one")
    t.translate_incoming("[A] one")   # 重試同一行:失敗那次不得已進上下文
    assert CONTEXT_HEADER not in fake.last_body["messages"][1]["content"]


def test_history_caps_at_context_lines():
    from src.translator import CONTEXT_LINES
    fake = FakeHttpxClient()
    t = _make(fake)
    for i in range(CONTEXT_LINES + 3):
        t.translate_incoming(f"[A] m{i}")
    user = fake.last_body["messages"][1]["content"]
    assert "[A] m0" not in user       # 最舊的已被擠出
    assert f"[A] m{CONTEXT_LINES - 1}" in user


def test_outgoing_gets_context_but_does_not_record():
    from src.translator import CONTEXT_HEADER, OUTGOING_TARGET_HEADER
    fake = FakeHttpxClient()
    t = _make(fake)
    t.translate_incoming("[A] want to trade?")
    t.translate_outgoing("好啊")
    user = fake.last_body["messages"][1]["content"]
    assert CONTEXT_HEADER in user
    assert "[A] want to trade?" in user
    assert OUTGOING_TARGET_HEADER in user
    assert user.endswith("好啊")
    assert len(t._history) == 1       # 發話不寫入上下文(遊戲回顯後由收訊記錄)


def test_incoming_system_mentions_context_rules():
    system = build_incoming_system("繁體中文（台灣）")
    assert "對話上下文" in system
    assert "無關" in system            # 混雜多組對話時忽略無關內容的規則


def test_meta_output_rejected_and_not_recorded_to_history():
    # 模型回覆格式說明（夾帶段落標記）＝沒在翻譯：當失敗處理，且不污染上下文
    fake = FakeHttpxClient(response=FakeResponse(
        content="Please provide the input in the following format:\n\n"
                "[對話上下文，僅供理解，不要翻譯]\n(Context here)\n\n"
                "[要發送的訊息]\n(Message here)"))
    t = _make(fake)
    with pytest.raises(RuntimeError, match="格式說明"):
        t.translate_outgoing("測試")
    with pytest.raises(RuntimeError, match="格式說明"):
        t.translate_incoming("[A] hi")
    assert len(t._history) == 0


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
