"""翻譯測試共用的假 client、假回應與請求輔助函式。"""
import json
from contextlib import contextmanager

import httpx

from src.translation.translator import Translator


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
