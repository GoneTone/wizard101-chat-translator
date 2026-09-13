"""共用翻譯 client：依設定的 provider 選擇後端（OpenAI 相容 /v1/chat/completions 或 Claude /v1/messages）。

收訊翻成使用者設定的目標語言（target_language），發話翻成 OUTGOING_LANGUAGE，
來源語言一律自動判斷；提示詞在 prompts.py，譯文後處理與品質判定在 postprocess.py。

失敗分三類：TranslatorOffline（可重試）、TranslatorConfigError（等使用者修設定）、
TranslatorBadOutput（譯文被截斷，重試無用、該行應跳過）。
list_models() 向端點取得模型清單，端點不支援時拋 TranslatorNoModelList。
上下文由呼叫端提供（見 context.py），本類別不持有狀態，可安全平行呼叫。
"""
import json
import re
import time

import anthropic
import httpx

from src.config import EFFORT_AUTO
from src.log import log
from src.translation.postprocess import (
    has_stray_latin,
    strip_invented_english,
    strip_think,
)
from src.translation.prompts import (
    CONTEXT_INTRO_INCOMING,
    CONTEXT_INTRO_OUTGOING,
    FEWSHOT_OUTGOING,
    OUTGOING_LANGUAGE,
    build_incoming_system,
    build_outgoing_system,
    build_system_message_system,
    build_turns,
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
_TIMEOUT = 60.0
# 譯文長度上限，目的是防 repetition loop 吃穿 _TIMEOUT 而非省 token：實測 temperature=0
# 遇到原文本身重複（「am chick um chick um chick」）會無限吐同一個字，180 秒仍不收斂，
# 整條收訊流程被誤判成「翻譯伺服器離線」並卡在該行重試。最壞譯文實測約 80 token。
_MAX_TOKENS = 512
# 思考模式下 <think>…</think> 區塊本身就會吃掉數百 token，上限需放寬才不會砍在譯文之前。
_MAX_TOKENS_THINKING = 2048
TEST_SAMPLE = "[Tester] Hello! How are you?"  # 測試連線用固定原文


class TranslatorError(Exception):
    """翻譯請求失敗的共同基底：`status` 是 HTTP 狀態碼（連線層失敗為 None），
    `detail` 是 API 回應裡的說明或連線失敗原因（見 error_detail），
    UI 與 log 都直接拿它顯示，不再只靠狀態碼猜使用者該檢查什麼。"""

    def __init__(self, detail: str = "", status: int | None = None):
        self.status = status
        self.detail = detail
        if status is not None and detail:
            message = f"HTTP {status}: {detail}"
        elif status is not None:
            message = f"HTTP {status}"
        else:
            message = detail
        super().__init__(message)


class TranslatorOffline(TranslatorError):
    """可重試的翻譯失敗：連線失敗、逾時、429、5xx。"""


class TranslatorBadOutput(Exception):
    """不可重試的輸出異常：譯文在 max_tokens 被截斷（模型 repetition loop 或脫稿長篇）。
    temperature=0 下重試必得同一結果，呼叫端應跳過該行而非留在佇列重試。"""


_TRUNCATED_SAMPLE_CHARS = 80  # 截斷樣本長度：足以看出是不是同一個字重複


def _truncated(max_tokens: int, completion_tokens, sample: str) -> TranslatorBadOutput:
    """組出帶診斷資訊的截斷例外。樣本與 token 數是為了讓使用者從 app.log 就能分辨
    「模型陷入 repetition loop」與「譯文真的過長、上限誤砍」，兩者的處置完全不同。"""
    head = sample[:_TRUNCATED_SAMPLE_CHARS].replace("\n", " ")
    return TranslatorBadOutput(
        f"output truncated at max_tokens={max_tokens}, "
        f"completion_tokens={completion_tokens}, sample={head!r}")


class TranslatorConfigError(TranslatorError):
    """設定錯誤：4xx（金鑰無效、模型不存在、參數不被接受等）。
    可重試 —— pool 以固定的 CONFIG_ERROR_INTERVAL 間隔持續重試，
    使用者於執行期間修正 config.json 後即自動恢復，不必重啟程式。"""


class TranslatorNoModelList(Exception):
    """此端點不提供模型清單（/v1/models 回 404／405，或回應缺 data 陣列）。
    與 TranslatorConfigError 的 404（模型不存在）是兩回事：這裡只代表「問不到清單」，
    使用者仍可自行輸入模型名稱正常翻譯。"""


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
# 長度上限兩個名字互換而非拿掉：它是防 repetition loop 的保險（見 _MAX_TOKENS）。
_TOKEN_LIMIT_PARAMS = {"max_tokens": "max_completion_tokens",
                       "max_completion_tokens": "max_tokens"}


def rejected_parameter(detail: str) -> str | None:
    """從 400 的說明文字抓出被拒的參數名；認不出來回 None。"""
    match = _REJECTED_PARAM.search(detail)
    if match is None:
        return None
    return match.group(1) or match.group(2)


class _BaseClient:
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


class _OpenAICompatClient(_BaseClient):
    """OpenAI 相容端點（ChatGPT 官方與自訂伺服器共用）：打 /v1/chat/completions。

    哪些參數能帶因模型而異（gpt-4o-mini 不認 reasoning_effort，GPT-5 不認 max_tokens
    與 temperature），靠模型名稱猜規則追不上改版，改成聽伺服器的：400 指名某個參數
    就拿掉它重送一次，並記在這個 client 上，之後的請求不再帶。

    official＝OpenAI 官方端點的起手式：長度上限用 max_completion_tokens、不帶
    temperature、停用思考只帶 reasoning_effort（未知欄位嚴格回 400）。自架後端
    （vLLM／Ollama／LM Studio）多半只認 max_tokens，temperature=0 也是為了它們的
    重現性，故起手維持原樣。"""

    def __init__(self, base_url: str, model: str, api_key: str = "",
                 thinking: bool = True, timeout: float = _TIMEOUT, client=None,
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

    def _body(self, system: str, turns: list[dict], max_tokens: int) -> dict:
        body = {
            "model": self._model,
            "messages": [{"role": "system", "content": system}, *turns],
            self._token_param: max_tokens,
        }
        if not self._official:
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

    def chat(self, system: str, turns: list[dict]) -> str:
        max_tokens = _MAX_TOKENS_THINKING if self._thinking else _MAX_TOKENS
        while True:
            body = self._body(system, turns, max_tokens)
            try:
                resp = self._client.post("/v1/chat/completions", json=body)
            except httpx.HTTPError as exc:
                raise TranslatorOffline(_one_line(str(exc))) from exc
            error = _status_error(resp.status_code, error_detail(resp.text))
            if error is None:
                break
            param = rejected_parameter(error.detail) if error.status == 400 else None
            if param is None or param not in body or not self._learn_rejection(param):
                raise error
            # 每輪都拿掉（或換掉）一個 body 裡確實有的參數，且同一個名字不會試第二次
            log(f"[translate] endpoint rejected parameter {param} (model={self._model}); "
                f"retrying without it")
        resp.raise_for_status()
        data = resp.json()
        choice = data["choices"][0]
        content = choice["message"]["content"]
        # 部分後端不回 finish_reason，缺欄位一律視為正常結束、不誤判成截斷
        if choice.get("finish_reason") == "length":
            raise _truncated(max_tokens,
                             (data.get("usage") or {}).get("completion_tokens"),
                             content or "")
        return strip_think(content).strip()

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


def _anthropic_detail(exc: anthropic.APIStatusError) -> str:
    """SDK 已把 body 解析成 dict（exc.body），取不到說明時退回 SDK 自己組的訊息。"""
    message = _message_of(exc.body)
    return _one_line(message if message is not None else exc.message)


class _ClaudeClient(_BaseClient):
    """Claude 官方 API（anthropic SDK）：打 /v1/messages。
    Claude 5 系不接受 temperature（會 400），也沒有「完全不思考」這個選項：
    思考深度改由 effort 控制，EFFORT_AUTO 時連 output_config 都不帶、維持模型
    預設（adaptive）。刻意不走 thinking={"type": "disabled"} —— 那在 Opus 5 會把
    <thinking> 標籤漏進回應，而 strip_think 只認 <think>。"""

    def __init__(self, model: str, api_key: str, effort: str = EFFORT_AUTO,
                 timeout: float = _TIMEOUT, client=None):
        self._client = client if client is not None else anthropic.Anthropic(
            api_key=api_key, timeout=timeout)
        self._model = model
        self._effort = effort

    def chat(self, system: str, turns: list[dict]) -> str:
        params = {"model": self._model, "max_tokens": _MAX_TOKENS_THINKING,
                  "system": system, "messages": turns}
        if self._effort != EFFORT_AUTO:
            params["output_config"] = {"effort": self._effort}
        try:
            resp = self._client.messages.create(**params)
        except anthropic.APIConnectionError as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        except anthropic.APIStatusError as exc:
            error = _status_error(exc.status_code, _anthropic_detail(exc))
            if error is None:
                raise
            raise error from exc
        content = "".join(b.text for b in resp.content if b.type == "text")
        if resp.stop_reason == "max_tokens":
            usage = getattr(resp, "usage", None)
            raise _truncated(_MAX_TOKENS_THINKING,
                             getattr(usage, "output_tokens", None), content)
        return strip_think(content).strip()

    def list_models(self) -> list[str]:
        try:
            page = self._client.models.list()  # SDK 自動翻頁，直接迭代即可
        except anthropic.APIConnectionError as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        except anthropic.APIStatusError as exc:
            error = _model_list_error(exc.status_code, _anthropic_detail(exc))
            if error is None:
                raise
            raise error from exc
        return sorted(m.id for m in page)


# 欄位一律給預設值：呼叫端（Translator、list_models）直接展開某一家的設定，
# 而每家有的欄位不同（見 config.API_PROFILE_FIELDS）。
def _build_client(provider: str = "custom", base_url: str = "", model: str = "",
                  api_key: str = "", thinking: bool = True, effort: str = EFFORT_AUTO,
                  timeout: float = _TIMEOUT, client=None):
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


class Translator:
    """共用翻譯 client：依 provider 選擇後端，收訊/發話介面不變。

    `**api` 是某一家服務商的設定（provider、model、api_key…；每家欄位不同，見
    config.API_PROFILE_FIELDS），呼叫端直接把 active_api(cfg) 展開進來，缺的欄位
    由 _build_client 補預設值。`timeout`／`client` 供測試注入假 client。"""

    def __init__(self, *, target_language: str, timeout: float = _TIMEOUT,
                 client=None, **api):
        self._impl = _build_client(**api, timeout=timeout, client=client)
        self._target_language = target_language

    def reconfigure(self, *, target_language: str, **api) -> None:
        """設定變更後就地重建後端 client（呼叫端不需換 Translator 實例）。
        舊 client 先關：否則每改一次設定就多留一個連線池。"""
        self._impl.close()
        self._impl = _build_client(**api)
        self._target_language = target_language

    def close(self) -> None:
        """釋放後端的連線池（一次性用途如測試連線，用完即關）。"""
        self._impl.close()

    @property
    def target_language(self) -> str:
        """目前的目標語言。呼叫端要判斷譯文品質時需要它（見 has_stray_latin）。"""
        return self._target_language

    def _chat(self, kind: str, system: str, turns: list[dict], *,
              source: str, context_lines: int, strip: bool = False) -> str:
        """打一次翻譯請求，回傳最終譯文並記錄一行診斷。

        三個方向共用的唯一成功路徑 log 點 —— 使用者匯出 app.log 後，能把每則原文與
        實際譯文並排對照（messages.log 只留原文，不留譯文）。失敗分支不在這裡記錄：
        例外往上拋，由 pool 依重試結果記錄（見 translation.pool）。
        """
        started = time.monotonic()
        translated = self._impl.chat(system, turns)
        if strip:
            translated = strip_invented_english(source, translated)
        log(f"[translate] {kind} done in {time.monotonic() - started:.1f}s "
            f"(model={self._impl.model}, ctx={context_lines}): "
            f"source={source!r} translated={translated!r}")
        return translated

    def translate_incoming(self, text: str, context: list[str]) -> str:
        """收訊：把遊戲聊天（任何語言）翻成使用者設定的目標語言。
        context 為該行之前的原文行，由呼叫端依讀取順序維護（見 ChatContext）。"""
        return self._chat(
            "incoming",
            build_incoming_system(self._target_language),
            build_turns(context, text, CONTEXT_INTRO_INCOMING),
            source=text, context_lines=len(context), strip=True)

    def translate_system_message(self, text: str) -> str:
        """系統訊息：把遊戲系統通知（任何語言）翻成使用者設定的目標語言。

        刻意不吃 context：系統訊息彼此獨立，8 行的上下文窗會被掉寶洗光、玩家對話失去
        語境；純函式化的呼叫也是譯文快取正確性的前提（見 translation.cache）。

        譯文落回英文時重譯一次（見 has_stray_latin）：裸名詞特別容易被改用官方英文名，
        實測重譯救得回約三分之一，救不回的（音譯玩家名）照樣回傳，呼叫端負責不寫進快取。
        只有這條路徑重譯：收訊有完整句子語境、實測不會落回英文。"""
        translated = self._system_message_once(text)
        if has_stray_latin(text, translated, self._target_language):
            log(f"[translate] system message is not in the target language, retrying "
                f"strictly: source={text!r} translated={translated!r}")
            translated = self._system_message_once(text, strict=True)
            if has_stray_latin(text, translated, self._target_language):
                log(f"[translate] strict retry is still not in the target language, "
                    f"using it as is: source={text!r} translated={translated!r}")
        return translated

    def _system_message_once(self, text: str, strict: bool = False) -> str:
        return self._chat(
            "system message (strict retry)" if strict else "system message",
            build_system_message_system(self._target_language, strict=strict),
            [{"role": "user", "content": text}],
            source=text, context_lines=0, strip=True)

    def translate_outgoing(self, text: str, context: list[str]) -> str:
        """發話：把玩家輸入（任何語言）翻成遊戲聊天語言（固定）。
        發話內容不寫入上下文 —— 送出後遊戲會回顯成聊天行，由收訊路徑記錄。
        few-shot 一律帶：曾只在無上下文時帶，但遊戲內幾乎永遠有上下文，實測模型會把
        「不好意思我英文不好，用翻譯器」當成對它說的話回「No worries, I'll help you out!」，
        而該回覆會被原樣送進遊戲聊天。"""
        return self._chat(
            "outgoing",
            build_outgoing_system(OUTGOING_LANGUAGE),
            build_turns(context, text, CONTEXT_INTRO_OUTGOING,
                        examples=FEWSHOT_OUTGOING),
            source=text, context_lines=len(context))


def list_models(api: dict, client=None) -> list[str]:
    """取得端點上可用的模型 ID（已排序）。api 為設定表單當下的值，與翻譯走同一條分派。
    端點不提供清單時拋 TranslatorNoModelList —— 呼叫端應提示改為自行輸入模型名稱。"""
    impl = _build_client(**api, timeout=_TIMEOUT, client=client)
    try:
        return impl.list_models()
    finally:
        impl.close()


def test_translate(api: dict, target_language: str) -> str:
    """測試連線：用表單當下的 api 設定實際翻一句固定文字，與正式翻譯同一條路。"""
    translator = Translator(**api, target_language=target_language)
    try:
        return translator.translate_incoming(TEST_SAMPLE, [])
    finally:
        translator.close()
