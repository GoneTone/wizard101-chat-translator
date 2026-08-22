import json

import httpx
import pytest

from src.translator import (
    OUTGOING_LANGUAGE, Translator, build_incoming_system, build_outgoing_system,
)

TARGET = "繁體中文（台灣）"


def make_translator(handler, target_language=TARGET) -> Translator:
    transport = httpx.MockTransport(handler)
    client = httpx.Client(base_url="http://test", transport=transport)
    return Translator(base_url="http://test", model="test-model",
                      target_language=target_language, client=client)


def ok_response(content: str) -> httpx.Response:
    return httpx.Response(200, json={"choices": [{"message": {"content": content}}]})


def test_incoming_sends_model_and_target_language_prompt():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["body"] = json.loads(request.content)
        return ok_response("  [A] 你好  ")

    t = make_translator(handler)
    assert t.translate_incoming("[A] hello") == "[A] 你好"  # 去頭尾空白
    assert captured["url"].endswith("/v1/chat/completions")
    assert captured["body"]["model"] == "test-model"
    assert captured["body"]["messages"][0] == {"role": "system", "content": build_incoming_system(TARGET)}
    assert captured["body"]["messages"][1] == {"role": "user", "content": "[A] hello"}


def test_incoming_prompt_uses_configured_language_not_hardcoded():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["body"] = json.loads(request.content)
        return ok_response("[A] こんにちは")

    make_translator(handler, target_language="日本語").translate_incoming("[A] hi")
    sys_prompt = captured["body"]["messages"][0]["content"]
    assert "日本語" in sys_prompt
    assert "繁體中文" not in sys_prompt  # 目標語言不寫死


def test_outgoing_uses_game_language_prompt():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["body"] = json.loads(request.content)
        return ok_response("wanna do a dungeon?")

    t = make_translator(handler)
    assert t.translate_outgoing("要不要打副本?") == "wanna do a dungeon?"
    assert captured["body"]["messages"][0] == {"role": "system", "content": build_outgoing_system(OUTGOING_LANGUAGE)}
    assert OUTGOING_LANGUAGE in captured["body"]["messages"][0]["content"]


def test_build_incoming_system_inserts_language():
    assert "Español" in build_incoming_system("Español")


def test_thinking_true_default_omits_disable_params():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["body"] = json.loads(request.content)
        return ok_response("x")

    make_translator(handler).translate_incoming("hi")  # 預設 thinking=True
    for k in ("reasoning_effort", "chat_template_kwargs", "think", "enable_thinking"):
        assert k not in captured["body"]


def test_thinking_false_adds_disable_params():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["body"] = json.loads(request.content)
        return ok_response("x")

    transport = httpx.MockTransport(handler)
    client = httpx.Client(base_url="http://test", transport=transport)
    Translator(base_url="http://test", model="m", target_language=TARGET,
               thinking=False, client=client).translate_outgoing("哈囉")
    b = captured["body"]
    assert b["reasoning_effort"] == "none"
    assert b["chat_template_kwargs"] == {"enable_thinking": False}
    assert b["think"] is False
    assert b["enable_thinking"] is False


def test_strips_think_block_from_incoming_output():
    t = make_translator(lambda req: ok_response("<think>先想想怎麼翻</think>\n[A] 你好，世界"))
    assert t.translate_incoming("hi") == "[A] 你好，世界"


def test_strips_think_block_from_outgoing_output():
    t = make_translator(lambda req: ok_response("<think>reasoning here</think>hello there"))
    assert t.translate_outgoing("嗨") == "hello there"


def test_server_error_raises_http_error():
    t = make_translator(lambda req: httpx.Response(500, text="boom"))
    with pytest.raises(httpx.HTTPError):
        t.translate_incoming("hi")


def test_api_key_sets_authorization_header():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["auth"] = request.headers.get("authorization")
        return ok_response("x")

    transport = httpx.MockTransport(handler)
    client = httpx.Client(base_url="http://test", transport=transport,
                          headers={"Authorization": "Bearer sk-123"})
    t = Translator(base_url="http://test", model="m", target_language=TARGET,
                   api_key="sk-123", client=client)
    t.translate_incoming("hi")
    assert captured["auth"] == "Bearer sk-123"


def test_default_client_construction_with_api_key():
    t = Translator(base_url="http://myserver", model="m", target_language=TARGET, api_key="sk-abc")
    assert t._client.headers["authorization"] == "Bearer sk-abc"
    assert str(t._client.base_url).rstrip("/") == "http://myserver"
    assert t._client.timeout.read == 10.0


def test_default_client_construction_without_api_key():
    t = Translator(base_url="http://myserver", model="m", target_language=TARGET)
    assert "authorization" not in t._client.headers
