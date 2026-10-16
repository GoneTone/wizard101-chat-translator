"""各服務商的 HTTP 客戶端：OpenAI 相容 /v1/chat/completions（串流 SSE）與 Claude /v1/messages，
以及 HTTP 錯誤的解析與分類、請求逾時與 token 上限、可從別的執行緒中斷請求的 RequestHandle。
Translator（translator.py）只透過 build_client() 取得客戶端，不碰各家協定細節。"""
import json
import re
import threading
from contextlib import contextmanager

import httpx

from src.config.services import EFFORT_AUTO
from src.logs import log
from src.translation.errors import (
    TranslatorBadOutput,
    TranslatorCancelled,
    TranslatorConfigError,
    TranslatorError,
    TranslatorNoModelList,
    TranslatorOffline,
)
from src.translation.postprocess import (
    strip_think,
)

# thinking=False 時併入請求 body 的停用參數，涵蓋常見後端（伺服器通常忽略不認得的欄位）。
_DISABLE_THINKING = {
    "reasoning_effort": "none",                        # OpenAI o 系 / 相容
    "chat_template_kwargs": {"enable_thinking": False},  # vLLM / SGLang + Qwen3
    "think": False,                                    # Ollama
    "enable_thinking": False,                          # 部分伺服器吃頂層
}


# OpenAI 官方端點對未知欄位嚴格回 400，只能帶它自己認得的停用參數。
_DISABLE_THINKING_OPENAI = {"reasoning_effort": "none"}


OPENAI_BASE_URL = "https://api.openai.com"  # ChatGPT preset 固定官方端點


TIMEOUT = 60.0


# 譯文長度上限，目的是防 repetition loop 吃穿 TIMEOUT 而非省 token：實測 temperature=0
# 遇到原文本身重複（「am chick um chick um chick」）會無限吐同一個字，180 秒仍不收斂，
# 整條收訊流程被誤判成「翻譯伺服器離線」並卡在該行重試。最壞譯文實測約 80 token。
MAX_TOKENS = 512


# 思考模式下推理本身就會吃掉大量 token，上限需放寬才不會砍在譯文之前：實測 qwen3.6-35b-a3b
# 光推理就用完 2048，每句都被截斷、譯文是空的。
MAX_TOKENS_THINKING = 4096


# 區域翻譯固定用放寬的上限：一次可能是一整頁任務書的所有行一起送出，
# 思考模式下 <think> 區塊還要跟譯文搶同一個預算；上限依然存在是為了
# 界住 repetition loop（見 MAX_TOKENS 的說明），不是為了省 token。
MAX_TOKENS_REGION = 4096


_TRUNCATED_SAMPLE_CHARS = 80  # 截斷樣本長度：足以看出是不是同一個字重複


def _truncated(max_tokens: int, completion_tokens, sample: str) -> TranslatorBadOutput:
    """組出帶診斷資訊的截斷例外。樣本與 token 數是為了讓使用者從 app.log 就能分辨
    「模型陷入 repetition loop」與「譯文真的過長、上限誤砍」，兩者的處置完全不同。"""
    head = sample[:_TRUNCATED_SAMPLE_CHARS].replace("\n", " ")
    return TranslatorBadOutput(
        f"output truncated at max_tokens={max_tokens}, "
        f"completion_tokens={completion_tokens}, sample={head!r}")


_HTML_TAG = re.compile(r"<[^>]+>")


def _message_of(body) -> str | None:
    """從已解析的錯誤 body 取說明文字：OpenAI／Anthropic 都是 error.message，
    部分自架後端把 error 或 message 直接放字串。"""
    if not isinstance(body, dict):
        return None
    error = body.get("error")
    if isinstance(error, dict) and isinstance(error.get("message"), str):
        return error["message"]
    if isinstance(error, str):
        return error
    if isinstance(body.get("message"), str):
        return body["message"]
    return None


def _one_line(text: str) -> str:
    """折成單行（多行 body、縮排的 JSON）；不截斷 —— 訊息尾端常是說明網址。"""
    return " ".join(text.split())


def error_detail(text: str) -> str:
    """把錯誤回應的原始 body 整理成可直接顯示的一句話：JSON 取 error.message 之類的
    欄位，HTML 錯誤頁去標籤，其餘照原文；一律折成單行。"""
    try:
        message = _message_of(json.loads(text))
    except ValueError:
        message = None
    if message is None:
        message = _HTML_TAG.sub(" ", text)
    return _one_line(message)


def _status_error(status: int, detail: str = "") -> TranslatorError | None:
    """翻譯請求的 HTTP 狀態碼映射成例外（None＝可繼續解析回應）；兩種後端共用，判定一致。
    4xx 一律當設定錯誤：400 多半是模型不吃某個參數，改設定才會好。"""
    if status == 429 or status >= 500:
        return TranslatorOffline(detail, status=status)
    if status >= 400:
        return TranslatorConfigError(detail, status=status)
    return None


def _model_list_error(status: int, detail: str = "") -> Exception | None:
    """模型清單請求的狀態碼對應：404／405 是「端點不提供清單」而非設定錯誤，
    其餘沿用翻譯請求的判定。"""
    if status in (404, 405):
        return TranslatorNoModelList(f"HTTP {status}")
    return _status_error(status, detail)


# OpenAI 官方 400 指名參數的三種寫法：非推理模型不認 reasoning_effort、
# GPT-5／o 系列不認 max_tokens 與非預設 temperature。
_REJECTED_PARAM = re.compile(
    r"Unrecognized request argument supplied: (\w+)"
    r"|Unsupported (?:parameter|value): '(\w+)'")


def rejected_parameter(detail: str) -> str | None:
    """從 400 的說明文字抓出被拒的參數名；認不出來回 None。"""
    match = _REJECTED_PARAM.search(detail)
    if match is None:
        return None
    return match.group(1) or match.group(2)


class RequestHandle:
    """可從別的執行緒撤銷的進行中請求。

    兩個後端都走串流：請求送出後把回應的 `close` 掛上來（`_attach`），`cancel()`
    從主執行緒關掉它，伺服器端隨即停止生成、只計已產生的 token —— 非串流請求做不到這點
    （斷線後伺服器仍會生成完整段並全額計費）。請求正常結束後 `_detach`，之後再 `cancel()`
    只立旗標、不會去關一個已經結束的回應。還沒送出就被取消的請求根本不會送。"""

    def __init__(self):
        self._lock = threading.Lock()
        self._close = None
        self._cancelled = False

    @property
    def cancelled(self) -> bool:
        return self._cancelled

    def cancel(self) -> None:
        with self._lock:
            self._cancelled = True
            close, self._close = self._close, None
        if close is not None:
            try:
                close()
            except Exception as exc:
                log(f"[translate] closing a cancelled request failed: {type(exc).__name__}: {exc}")

    def _attach(self, close) -> None:
        with self._lock:
            if self._cancelled:
                raise TranslatorCancelled()
            self._close = close

    def _detach(self) -> None:
        with self._lock:
            self._close = None


@contextmanager
def _cancellable(cancel: RequestHandle | None, close):
    """把已開啟的串流回應交給 cancel 管：期間任何例外若是取消造成的一律轉成
    TranslatorCancelled（斷線在 httpx／SDK 那層冒出來的例外型別不一，靠旗標分辨）。"""
    if cancel is None:
        yield
        return
    cancel._attach(close)
    try:
        yield
    except Exception as exc:
        if cancel.cancelled:
            raise TranslatorCancelled() from exc
        raise
    finally:
        cancel._detach()


class BaseClient:
    """兩種後端共用的骨架：模型 ID 與連線池釋放。子類別各自實作 chat／list_models。"""

    _client = None
    _model = ""

    @property
    def model(self) -> str:
        """目前使用的模型 ID（診斷 log 用）。"""
        return self._model

    def close(self) -> None:
        """釋放連線池。測試注入的假 client 不一定有 close，沒有就略過。"""
        close = getattr(self._client, "close", None)
        if close is not None:
            close()


# 長度上限兩個名字互換而非拿掉：它是防 repetition loop 的保險（見 MAX_TOKENS）。
_TOKEN_LIMIT_PARAMS = {"max_tokens": "max_completion_tokens",
                       "max_completion_tokens": "max_tokens"}


class _OpenAICompatClient(BaseClient):
    """OpenAI 相容端點（ChatGPT 官方與自訂伺服器共用）：打 /v1/chat/completions。

    哪些參數能帶因模型而異（gpt-4o-mini 不認 reasoning_effort，GPT-5 不認 max_tokens
    與 temperature），靠模型名稱猜規則追不上改版，改成聽伺服器的：400 指名某個參數
    就拿掉它重送一次，並記在這個 client 上，之後的請求不再帶。

    official＝OpenAI 官方端點的起手式：長度上限用 max_completion_tokens、不帶
    temperature、停用思考只帶 reasoning_effort（未知欄位嚴格回 400）。自架後端
    （vLLM／Ollama／LM Studio）多半只認 max_tokens，temperature=0 也是為了它們的
    重現性，故起手維持原樣。"""

    def __init__(self, base_url: str, model: str, api_key: str = "",
                 thinking: bool = True, timeout: float = TIMEOUT, client=None,
                 official: bool = False):
        self._official = official
        if client is not None:
            self._client = client
        else:
            headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
            self._client = httpx.Client(base_url=base_url, headers=headers, timeout=timeout)
        self._model = model
        self._thinking = thinking
        self._token_param = "max_completion_tokens" if official else "max_tokens"
        self._dropped: set[str] = set()
        self._rejected_token_params: set[str] = set()

    def _body(self, system: str, turns: list[dict], max_tokens: int,
              deterministic: bool = True) -> dict:
        body = {
            "model": self._model,
            "messages": [{"role": "system", "content": system}, *turns],
            self._token_param: max_tokens,
            "stream": True,
        }
        if self._official:
            # 串流預設不回 usage；截斷診斷要 completion_tokens。自架後端不一定認得這個欄位，
            # 為了不冒 400 的險只對官方端點帶。
            body["stream_options"] = {"include_usage": True}
        elif deterministic:
            body["temperature"] = 0
        if not self._thinking:
            body.update(_DISABLE_THINKING_OPENAI if self._official else _DISABLE_THINKING)
        for name in self._dropped:
            body.pop(name, None)
        return body

    def _learn_rejection(self, param: str) -> bool:
        """記住端點拒絕的參數，回傳下一次請求是否還有變化可試。
        長度上限兩個名字都被拒過就沒招了：再互換只會無限 ping-pong。"""
        if param in _TOKEN_LIMIT_PARAMS:
            self._rejected_token_params.add(param)
            if _TOKEN_LIMIT_PARAMS[param] in self._rejected_token_params:
                return False
            self._token_param = _TOKEN_LIMIT_PARAMS[param]
        else:
            self._dropped.add(param)
        return True

    def chat(self, system: str, turns: list[dict], max_tokens: int | None = None,
             cancel: RequestHandle | None = None, deterministic: bool = True) -> str:
        """打一次串流的 /v1/chat/completions，把 SSE 塊接回整段文字。
        串流是為了讓 `cancel` 能真的省 token（見 RequestHandle）；狀態碼錯誤在開頭就會回，
        跟非串流時一樣走 `_status_error` 與參數重送。
        `deterministic=False` 時自架端點不送 temperature（生成範例的重試要靠隨機性換結果）。"""
        if max_tokens is None:
            max_tokens = MAX_TOKENS_THINKING if self._thinking else MAX_TOKENS
        while True:
            body = self._body(system, turns, max_tokens, deterministic)
            try:
                content, finish_reason, usage = self._stream_once(body, cancel)
                break
            except TranslatorError as error:
                param = rejected_parameter(error.detail) if error.status == 400 else None
                if param is None or param not in body or not self._learn_rejection(param):
                    raise
            # 每輪都拿掉（或換掉）一個 body 裡確實有的參數，且同一個名字不會試第二次
            log(f"[translate] endpoint rejected parameter {param} (model={self._model}); "
                f"retrying without it")
        usage = usage or {}
        if "prompt_tokens" in usage:
            cached = (usage.get("prompt_tokens_details") or {}).get("cached_tokens")
            log(f"[translate] prompt cache: provider=openai-compatible model={self._model} "
                f"prompt={usage['prompt_tokens']} cached={cached}")
        # 部分後端不回 finish_reason，缺欄位一律視為正常結束、不誤判成截斷
        if finish_reason == "length":
            raise _truncated(max_tokens, usage.get("completion_tokens"), content)
        return strip_think(content).strip()

    def _stream_once(self, body: dict, cancel: RequestHandle | None
                     ) -> tuple[str, str | None, dict | None]:
        """送一次串流請求並讀完：回 (內容, finish_reason, usage)。
        狀態碼錯誤映射成 TranslatorError 拋出（4xx 在串流開頭就會回，跟非串流一樣），
        連線層例外一律 TranslatorOffline。"""
        try:
            with self._client.stream("POST", "/v1/chat/completions", json=body) as resp:
                if resp.status_code >= 400:
                    resp.read()
                    raise _status_error(resp.status_code, error_detail(resp.text))
                with _cancellable(cancel, resp.close):
                    return _read_sse(resp)
        except (httpx.HTTPError, httpx.StreamError) as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        except RuntimeError as exc:
            # 請求開頭讀到的設定剛被 reconfigure 換掉、舊 client 已關：當離線讓 pool 用新設定重試
            if getattr(self._client, "is_closed", False):
                raise TranslatorOffline(f"client closed by a settings change: {exc}") from exc
            raise

    def list_models(self) -> list[str]:
        try:
            resp = self._client.get("/v1/models")
        except httpx.HTTPError as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        error = _model_list_error(resp.status_code, error_detail(resp.text))
        if error is not None:
            raise error
        resp.raise_for_status()
        data = resp.json().get("data")
        if not isinstance(data, list):
            raise TranslatorNoModelList("response has no data array")
        return sorted(str(m["id"]) for m in data if isinstance(m, dict) and m.get("id"))


def _read_sse(resp) -> tuple[str, str | None, dict | None]:
    """讀完一條 chat completions 的 SSE 串流：回 (接起來的內容, finish_reason, usage)。
    每塊是 `data: {...}`，`data: [DONE]` 收尾；finish_reason 在最後一塊帶內容的 choice 上，
    usage（有帶 stream_options 才會有）另成一塊、choices 為空。"""
    parts: list[str] = []
    finish_reason = None
    usage = None
    for line in resp.iter_lines():
        if not line.startswith("data:"):
            continue
        data = line[len("data:"):].strip()
        if data == "[DONE]":
            break
        chunk = json.loads(data)
        choices = chunk.get("choices") or []
        if choices:
            piece = (choices[0].get("delta") or {}).get("content")
            if piece:
                parts.append(piece)
            if choices[0].get("finish_reason"):
                finish_reason = choices[0]["finish_reason"]
        if chunk.get("usage"):
            usage = chunk["usage"]
    return "".join(parts), finish_reason, usage


def _anthropic_detail(exc) -> str:
    """`anthropic.APIStatusError` → 一行錯誤說明。

    型別不寫進簽名：`anthropic` 已改成延遲 import（見 `_ClaudeClient.__init__`），
    而 3.11／3.12 會在函式定義時求值註解，寫了會 NameError。
    SDK 已把 body 解析成 dict（exc.body），取不到說明時退回 SDK 自己組的訊息。
    """
    message = _message_of(exc.body)
    return _one_line(message if message is not None else exc.message)


# Claude（尤其 Haiku）常不思考就在正文裡自我修正（先給譯文、再寫「Wait —」重來），
# 限定成單一 JSON 字串欄位後實測不再發生；其他服務商沒有這個症狀，不套用。
_CLAUDE_OUTPUT_FORMAT = {"type": "json_schema", "schema": {
    "type": "object", "properties": {"translation": {"type": "string"}},
    "required": ["translation"], "additionalProperties": False}}


class _ClaudeClient(BaseClient):
    """Claude 官方 API（anthropic SDK）：打 /v1/messages。
    Claude 5 系不接受 temperature（會 400），也沒有「完全不思考」這個選項：
    思考深度改由 effort 控制，EFFORT_AUTO 時連 output_config 都不帶、維持模型
    預設（adaptive）。刻意不走 thinking={"type": "disabled"} —— 那在 Opus 5 會把
    <thinking> 標籤漏進回應，而 strip_think 只認 <think>。
    回應一律走結構化輸出（_CLAUDE_OUTPUT_FORMAT），chat 解開 JSON 後回傳純文字。"""

    def __init__(self, model: str, api_key: str, effort: str = EFFORT_AUTO,
                 timeout: float = TIMEOUT, client=None):
        # anthropic 的 import 要約 0.7 秒（大量 pydantic model 定義），而只有選 Claude
        # 官方 API 的使用者需要它 —— 放在這裡，啟動路徑就完全不碰。
        import anthropic
        self._sdk = anthropic
        self._client = client if client is not None else anthropic.Anthropic(
            api_key=api_key, timeout=timeout)
        self._model = model
        self._effort = effort

    def chat(self, system: str, turns: list[dict], max_tokens: int | None = None,
             cancel: RequestHandle | None = None, deterministic: bool = True) -> str:
        """打一次串流的 /v1/messages 請求（SDK 的 `messages.stream` 幫忙累積成完整訊息），
        回傳文字內容。串流是為了讓 `cancel` 能真的省 token（見 RequestHandle）。
        `max_tokens` 未指定（None）時沿用聊天路徑的 `MAX_TOKENS_THINKING`——Claude 沒有
        「不思考」模式，思考深度改由 `effort` 控制，不是靠調這個參數；區域翻譯路徑會
        明確帶 `MAX_TOKENS_REGION`（一次可能送出一整頁的辨識行，預算得放寬），截斷判定
        也要用同一個值，否則沒超過真正上限的輸出會被誤判成截斷。
        `deterministic` 不適用（本來就不送 temperature），收下後忽略。"""
        limit = max_tokens if max_tokens is not None else MAX_TOKENS_THINKING
        # 斷點放在 system 結尾：之後的上下文每句都變，放在最後只會一直寫入、讀不到
        params = {"model": self._model, "max_tokens": limit,
                  "system": [{"type": "text", "text": system,
                              "cache_control": {"type": "ephemeral"}}],
                  "messages": turns,
                  "output_config": {"format": _CLAUDE_OUTPUT_FORMAT}}
        if self._effort != EFFORT_AUTO:
            params["output_config"]["effort"] = self._effort
        try:
            with (self._client.messages.stream(**params) as stream,
                  _cancellable(cancel, stream.close)):
                resp = stream.get_final_message()
        except (self._sdk.APIConnectionError, httpx.HTTPError, httpx.StreamError) as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        except self._sdk.APIStatusError as exc:
            error = _status_error(exc.status_code, _anthropic_detail(exc))
            if error is None:
                raise
            raise error from exc
        usage = getattr(resp, "usage", None)
        if usage is not None:
            log(f"[translate] prompt cache: provider=claude model={self._model} "
                f"read={usage.cache_read_input_tokens} write={usage.cache_creation_input_tokens} "
                f"uncached={usage.input_tokens}")
        # 文件只保證第一個 text 區塊是合法 JSON；實測此模式下也只會有一個 text 區塊
        content = next((b.text for b in resp.content if b.type == "text"), "")
        if resp.stop_reason == "max_tokens":
            raise _truncated(limit, getattr(usage, "output_tokens", None), content)
        try:
            text = json.loads(content)["translation"]
        except (ValueError, KeyError, TypeError) as exc:
            log(f"[translate] claude output is not the expected JSON: model={self._model} "
                f"stop_reason={resp.stop_reason} sample={content[:_TRUNCATED_SAMPLE_CHARS]!r}")
            raise TranslatorBadOutput(f"unexpected output (stop_reason={resp.stop_reason})") from exc
        return strip_think(text).strip()

    def list_models(self) -> list[str]:
        try:
            page = self._client.models.list()  # SDK 自動翻頁，直接迭代即可
        except self._sdk.APIConnectionError as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        except self._sdk.APIStatusError as exc:
            error = _model_list_error(exc.status_code, _anthropic_detail(exc))
            if error is None:
                raise
            raise error from exc
        return sorted(m.id for m in page)


# 欄位一律給預設值：呼叫端（Translator、list_models）直接展開某一家的設定，
# 而每家有的欄位不同（見 services.API_PROFILE_FIELDS）。
def build_client(provider: str = "custom", base_url: str = "", model: str = "",
                  api_key: str = "", thinking: bool = True, effort: str = EFFORT_AUTO,
                  timeout: float = TIMEOUT, client=None):
    if provider == "claude":
        return _ClaudeClient(model=model, api_key=api_key, effort=effort,
                             timeout=timeout, client=client)
    if provider == "openai":
        # 官方端點固定 base_url；請求 body 的差異見 _OpenAICompatClient
        return _OpenAICompatClient(base_url=OPENAI_BASE_URL, model=model, api_key=api_key,
                                   thinking=thinking, timeout=timeout, client=client,
                                   official=True)
    return _OpenAICompatClient(base_url=base_url, model=model, api_key=api_key,
                               thinking=thinking, timeout=timeout, client=client)
