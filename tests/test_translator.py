import json

import httpx
import pytest

from src.translator import EN_SYSTEM, ZH_SYSTEM, Translator, normalize_zh_punct


def make_translator(handler) -> Translator:
    transport = httpx.MockTransport(handler)
    client = httpx.Client(base_url="http://test", transport=transport)
    return Translator(base_url="http://test", model="test-model", client=client)


def ok_response(content: str) -> httpx.Response:
    return httpx.Response(200, json={"choices": [{"message": {"content": content}}]})


def test_to_zh_sends_model_and_system_prompt_and_returns_stripped():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["body"] = json.loads(request.content)
        return ok_response("  你好,朋友!  ")

    t = make_translator(handler)
    assert t.to_zh("hello friend!") == "你好，朋友！"  # strip + 標點正規化
    assert captured["url"].endswith("/v1/chat/completions")
    assert captured["body"]["model"] == "test-model"
    assert captured["body"]["messages"][0] == {"role": "system", "content": ZH_SYSTEM}
    assert captured["body"]["messages"][1] == {"role": "user", "content": "hello friend!"}


def test_to_en_uses_english_system_prompt():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["body"] = json.loads(request.content)
        return ok_response("wanna do a dungeon?")

    t = make_translator(handler)
    assert t.to_en("要不要打副本?") == "wanna do a dungeon?"
    assert captured["body"]["messages"][0] == {"role": "system", "content": EN_SYSTEM}


def test_server_error_raises_http_error():
    t = make_translator(lambda req: httpx.Response(500, text="boom"))
    with pytest.raises(httpx.HTTPError):
        t.to_zh("hi")


def test_api_key_sets_authorization_header():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["auth"] = request.headers.get("authorization")
        return ok_response("x")

    transport = httpx.MockTransport(handler)
    client = httpx.Client(base_url="http://test", transport=transport,
                          headers={"Authorization": "Bearer sk-123"})
    t = Translator(base_url="http://test", model="m", api_key="sk-123", client=client)
    t.to_zh("hi")
    assert captured["auth"] == "Bearer sk-123"


def test_default_client_construction_with_api_key():
    t = Translator(base_url="http://myserver", model="m", api_key="sk-abc")
    assert t._client.headers["authorization"] == "Bearer sk-abc"
    assert str(t._client.base_url).rstrip("/") == "http://myserver"
    assert t._client.timeout.read == 10.0


def test_default_client_construction_without_api_key():
    t = Translator(base_url="http://myserver", model="m")
    assert "authorization" not in t._client.headers


# --- normalize_zh_punct:譯文標點正規化(台灣全形慣例) ---
def test_normalize_converts_halfwidth_after_cjk():
    assert normalize_zh_punct("[A] 你好,朋友!要組隊嗎?") == "[A] 你好，朋友！要組隊嗎？"
    assert normalize_zh_punct("[A] 等等:先補血;再上") == "[A] 等等：先補血；再上"


def test_normalize_converts_period_and_ellipsis():
    assert normalize_zh_punct("[A] 好.") == "[A] 好。"
    assert normalize_zh_punct("[A] 讓我想想...") == "[A] 讓我想想……"


def test_normalize_converts_parens_around_cjk():
    assert normalize_zh_punct("[A] 走吧(快點)") == "[A] 走吧（快點）"


def test_normalize_keeps_english_numbers_emoticons():
    assert normalize_zh_punct("[A] hi, friend!") == "[A] hi, friend!"
    assert normalize_zh_punct("[A] 賣 1,000 金幣") == "[A] 賣 1,000 金幣"
    assert normalize_zh_punct("[A] 好喔 :)") == "[A] 好喔 :)"
    assert normalize_zh_punct("[A] 版本 3.5 出了") == "[A] 版本 3.5 出了"


def test_to_zh_applies_normalization():
    transport = httpx.MockTransport(
        lambda req: ok_response("[A] 你好,世界!"))
    client = httpx.Client(base_url="http://test", transport=transport)
    t = Translator(base_url="http://test", model="m", client=client)
    assert t.to_zh("[A] hello, world!") == "[A] 你好，世界！"
