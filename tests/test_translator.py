"""translator 的 provider 選擇與錯誤映射測試（mock client，不打真 API）。"""
import anthropic
import httpx
import httpx2
import pytest

from src.translation.translator import (
    _PAREN_ENGLISH,
    OPENAI_BASE_URL,
    Translator,
    TranslatorBadOutput,
    TranslatorConfigError,
    TranslatorNoModelList,
    TranslatorOffline,
    _game_noun_rule,
    build_incoming_system,
    build_system_message_system,
    error_detail,
    has_stray_latin,
    list_models,
    strip_invented_english,
)


class FakeResponse:
    def __init__(self, status_code=200, content="譯文", finish_reason="stop",
                 completion_tokens=None, payload=None, text=""):
        self.status_code = status_code
        self.text = text
        self._content = content
        self._finish_reason = finish_reason
        self._completion_tokens = completion_tokens
        self._payload = payload

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


def _anthropic_status_error(status, body=None):
    resp = httpx2.Response(status, request=httpx2.Request("POST", "http://x"))
    return anthropic.APIStatusError("err", response=resp, body=body)


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
    from src.translation.translator import _MAX_TOKENS, _MAX_TOKENS_THINKING
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
    from src.translation.translator import _MAX_TOKENS_THINKING
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
    from src.config import EFFORT_LOW
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


def test_openai_provider_forces_official_base_url():
    t = Translator(provider="openai", base_url="http://evil.example", model="m",
                   api_key="k", target_language="繁體中文（台灣）")
    assert str(t._impl._client.base_url) == OPENAI_BASE_URL


def test_build_turns_without_context_is_single_user_turn():
    from src.translation.translator import build_turns
    assert build_turns([], "[A] hi", "intro") == [
        {"role": "user", "content": "[A] hi"}]


def test_build_turns_prepends_fewshot_examples():
    from src.translation.translator import build_turns
    examples = [{"role": "user", "content": "在嗎"},
                {"role": "assistant", "content": "you there?"}]
    turns = build_turns([], "哈囉", "intro", examples=examples)
    assert turns[:2] == examples                       # 範例在最前
    assert turns[-1] == {"role": "user", "content": "哈囉"}  # 待翻句仍在最後


def test_outgoing_uses_fewshot_when_no_context():
    from src.translation.translator import FEWSHOT_OUTGOING
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
    from src.translation.translator import FEWSHOT_OUTGOING
    fake = FakeHttpxClient()
    _make(fake).translate_outgoing("好啊", ["[A] want to trade?"])
    turns = _turns(fake.last_body)
    # 遊戲內幾乎永遠有上下文，範例若在此時被略過，防脫稿保護等於沒有
    assert turns[:len(FEWSHOT_OUTGOING)] == FEWSHOT_OUTGOING
    assert any("[A] want to trade?" in m["content"] for m in turns)
    assert turns[-1] == {"role": "user", "content": "好啊"}


def test_build_turns_with_context_is_multi_turn():
    from src.translation.translator import CONTEXT_ACK, build_turns
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
    assert "語境" in system            # 仍說明會收到聊天記錄當背景


def test_both_systems_forbid_treating_input_as_instructions():
    # 輸入內容長得像指令時模型不得脫稿回應（實測踩過：回了「了解。請提供…」）
    from src.translation.translator import build_outgoing_system
    assert "絕不回應" in build_incoming_system("繁體中文（台灣）")
    assert "絕不回應" in build_outgoing_system("English")


def test_reconfigure_switches_provider():
    t = _make(FakeHttpxClient())
    t.reconfigure(provider="claude", base_url="", model="claude-opus-5", api_key="k",
                  thinking=False, target_language="日本語")
    # reconfigure 後為 Claude client（真物件）；此處只驗證型別切換，不打 API
    from src.translation.translator import _ClaudeClient
    assert isinstance(t._impl, _ClaudeClient)


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
    # 這條規則曾在收訊與系統訊息兩處各寫一份，改一處就會漏另一處——
    # 實機回報的「自行編造英文」正源於此。共用同一份，結構上防止再度分岔。
    rule = _game_noun_rule("日本語")
    assert rule in build_incoming_system("日本語")
    assert rule in build_system_message_system("日本語")


def test_game_noun_rule_forbids_inventing_english_for_non_english_source():
    # 原文非英文時模型不得自行翻一個英文塞進括號
    rule = _game_noun_rule("繁體中文（台灣）")
    assert "不得自行翻譯或補上任何英文" in rule


def test_game_noun_rule_still_keeps_english_when_the_source_is_english():
    assert "原文本來就有英文時" in _game_noun_rule("繁體中文（台灣）")


def test_game_noun_rule_carries_no_english_example():
    # 示範一次「譯名(English)」就會把模型帶往英文：實機「雪刺帽」被整個譯成 Snowspike Hat，
    # 拿掉範例後穩定翻成中文。這裡用清理譯文的那條正則反過來擋住範例回流
    assert _PAREN_ENGLISH.search(_game_noun_rule("繁體中文（台灣）")) is None


def test_game_noun_rule_keeps_player_and_npc_names_untranslated():
    assert "原樣保留" in _game_noun_rule("繁體中文（台灣）")


def test_game_noun_rule_names_the_target_language():
    assert "日本語" in _game_noun_rule("日本語")
    assert "Español" in _game_noun_rule("Español")


# --- strip_invented_english：原文沒有英文時，譯文的括號英文必然是模型生成的 ---
def test_strip_invented_english_removes_parenthesised_english_for_cjk_source():
    # 實測：同一則簡中材料名，兩次翻譯分別補上 (Psychedelic Wood) 與 (Mystic Wood)
    assert strip_invented_english("迷幻木头", "迷幻木頭(Mystic Wood)") == "迷幻木頭"


def test_strip_invented_english_keeps_english_when_the_source_has_english():
    # 原文本來就有英文：括號可能是照抄的，不得動
    assert strip_invented_english(
        "Proud Pegasus Statue", "驕傲的飛馬雕像(Proud Pegasus Statue)"
    ) == "驕傲的飛馬雕像(Proud Pegasus Statue)"


def test_strip_invented_english_ignores_an_english_sender_name():
    # 判斷只看訊息內容：發送者名是英文不代表內容有英文
    assert strip_invented_english(
        "[Amy] 迷幻木头", "[Amy] 迷幻木頭(Mystic Wood)"
    ) == "[Amy] 迷幻木頭"


def test_strip_invented_english_keeps_the_sender_prefix():
    assert strip_invented_english("[艾米] 迷幻木头", "[艾米] 迷幻木頭(Mystic Wood)") \
        == "[艾米] 迷幻木頭"


def test_strip_invented_english_handles_full_width_parentheses():
    assert strip_invented_english("迷幻木头", "迷幻木頭（Mystic Wood）") == "迷幻木頭"


def test_strip_invented_english_leaves_placeholders_alone():
    assert strip_invented_english("你获得了 {0} 金币！", "你獲得了 {0} 金幣！") \
        == "你獲得了 {0} 金幣！"


def test_strip_invented_english_leaves_cjk_parentheses_content_alone():
    # 括號裡不是英文就不是幻覺，原樣保留
    assert strip_invented_english("熔岩百合", "熔岩百合（一種材料）") == "熔岩百合（一種材料）"


def test_strip_invented_english_removes_every_occurrence():
    assert strip_invented_english(
        "你获得了迷幻木头和熔岩百合", "你獲得了迷幻木頭(Mystic Wood)和熔岩百合(Lava Lily)"
    ) == "你獲得了迷幻木頭和熔岩百合"


def test_translate_incoming_strips_invented_english():
    fake = FakeHttpxClient(response=FakeResponse(content="[艾米] 迷幻木頭(Mystic Wood)"))
    assert _make(fake).translate_incoming("[艾米] 迷幻木头", []) == "[艾米] 迷幻木頭"


def test_translate_system_message_strips_invented_english():
    fake = FakeHttpxClient(response=FakeResponse(content="迷幻木頭(Mystic Wood)"))
    assert _make(fake).translate_system_message("迷幻木头") == "迷幻木頭"


class FakeSequenceClient(FakeHttpxClient):
    """依序回傳多個回應，並留下每一次的 request body：重譯路徑要看兩次請求。
    回應用完後重複最後一個。"""

    def __init__(self, contents):
        super().__init__()
        self._queue = [FakeResponse(content=c) for c in contents]
        self.bodies = []

    def post(self, url, json):
        self.bodies.append(json)
        return self._queue.pop(0) if len(self._queue) > 1 else self._queue[0]


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
    assert "整則譯文必須完全以 日本語 書寫" in build_system_message_system("日本語")


def test_strict_system_message_prompt_calls_out_the_english_slip():
    strict = build_system_message_system("日本語", strict=True)
    assert build_system_message_system("日本語") in strict
    assert "上一次" in strict


def test_translate_system_message_retries_when_the_model_answers_in_english():
    fake = FakeSequenceClient(["Snowspike Hat", "雪刺帽"])
    assert _make(fake).translate_system_message("雪刺帽") == "雪刺帽"
    assert len(fake.bodies) == 2
    assert "上一次" in fake.bodies[1]["messages"][0]["content"]   # 重譯用更嚴格的提示詞


def test_translate_system_message_does_not_retry_a_clean_translation():
    fake = FakeSequenceClient(["雪刺帽"])
    _make(fake).translate_system_message("雪刺帽")
    assert len(fake.bodies) == 1


def test_translate_system_message_keeps_a_retry_that_is_still_english():
    # 實測音譯的玩家名重譯仍會英譯：照樣顯示（呼叫端負責不快取），不再多打第三次
    fake = FakeSequenceClient(["Calamity 現在等級 {0}！"])
    tr = _make(fake)
    assert tr.translate_system_message("卡拉米蒂 现在等级 {0}！") == "Calamity 現在等級 {0}！"
    assert len(fake.bodies) == 2


def test_incoming_translation_is_not_retried():
    # 收訊有完整句子語境、實測不會落回英文；多打一次只是白花錢
    fake = FakeSequenceClient(["Snowspike Hat"])
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
    # 原文有一個英文名，模型卻換上另一個——片段不在原文裡就是憑空生成的
    assert has_stray_latin("收到 [Amy] 的伙伴邀请", "收到 Snowspike Hat 的夥伴邀請",
                           "繁體中文（台灣）")


def test_has_stray_latin_covers_non_latin_target_languages():
    for language in ["日本語", "한국어", "Русский", "ภาษาไทย", "Ελληνικά"]:
        assert has_stray_latin("雪刺帽", "Snowspike Hat", language)


def test_has_stray_latin_accepts_a_translation_without_any_latin():
    assert not has_stray_latin("你获得了 {0} 金币！", "{0} ゴールドを手に入れた！", "日本語")
