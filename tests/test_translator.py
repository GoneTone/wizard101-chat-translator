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
    t = _make(fake)
    t.translate_outgoing("在嗎")   # 無背景上下文：帶 few-shot 強制翻譯模式
    turns = _turns(fake.last_body)
    assert turns[:len(FEWSHOT_OUTGOING)] == FEWSHOT_OUTGOING
    assert turns[-1] == {"role": "user", "content": "在嗎"}
    # 範例中示範「像指令的訊息也照翻」，直接對抗脫稿
    assert any("提供" in m["content"] for m in FEWSHOT_OUTGOING if m["role"] == "user")


def test_outgoing_skips_fewshot_when_context_present():
    # 有背景上下文時不加 few-shot：多輪結構已足夠,避免範例與背景 turn 交錯干擾弱模型
    from src.translator import FEWSHOT_OUTGOING
    fake = FakeHttpxClient()
    t = _make(fake)
    t.translate_incoming("[A] want to trade?")
    t.translate_outgoing("好啊")
    turns = _turns(fake.last_body)
    assert turns[0] != FEWSHOT_OUTGOING[0]                 # 不以範例開頭
    assert any("[A] want to trade?" in m["content"] for m in turns)  # 背景仍在
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


def test_incoming_history_feeds_next_translation():
    fake = FakeHttpxClient()
    t = _make(fake)
    t.translate_incoming("[A] one")
    assert _turns(fake.last_body) == [{"role": "user", "content": "[A] one"}]
    t.translate_incoming("[B] two")
    turns = _turns(fake.last_body)
    assert len(turns) == 3                         # user(背景)+assistant(ack)+user(待翻)
    assert "[A] one" in turns[0]["content"]        # 上一行成為背景
    assert turns[-1] == {"role": "user", "content": "[B] two"}


def test_failed_translation_not_recorded_to_history():
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
    assert _turns(fake.last_body) == [{"role": "user", "content": "[A] one"}]


def test_history_caps_at_context_lines():
    from src.translator import CONTEXT_LINES
    fake = FakeHttpxClient()
    t = _make(fake)
    for i in range(CONTEXT_LINES + 3):
        t.translate_incoming(f"[A] m{i}")
    background = _turns(fake.last_body)[0]["content"]
    assert "[A] m0" not in background       # 最舊的已被擠出
    assert f"[A] m{CONTEXT_LINES - 1}" in background


def test_outgoing_gets_context_but_does_not_record():
    fake = FakeHttpxClient()
    t = _make(fake)
    t.translate_incoming("[A] want to trade?")
    t.translate_outgoing("好啊")
    turns = _turns(fake.last_body)
    assert any("[A] want to trade?" in m["content"] for m in turns)  # 背景在某一輪
    assert turns[-1] == {"role": "user", "content": "好啊"}  # 待翻句仍在最後
    assert len(t._history) == 1       # 發話不寫入上下文(遊戲回顯後由收訊記錄)


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
