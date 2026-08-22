"""共用翻譯 client：依設定的 provider 選擇後端（OpenAI 相容 /v1/chat/completions 或 Claude /v1/messages）。

收訊:來源語言自動判斷 → 翻成使用者設定的目標語言(target_language)。
發話:來源語言自動判斷 → 翻成遊戲聊天語言(OUTGOING_LANGUAGE,固定)。
提示詞依語言參數動態建構,程式碼不綁定特定語言。
"""
import re

import anthropic
import httpx

# 發話固定翻成的語言(遊戲聊天使用的語言);為固定產品設定,不進 config。
OUTGOING_LANGUAGE = "English"


def build_incoming_system(target_language: str) -> str:
    """建構收訊翻譯的 system 提示:把聊天內容翻成 target_language(來源語言自動判斷)。"""
    return (
        f"你是一個專業的翻譯員，負責將線上遊戲 Wizard101 的聊天對話文本"
        f"（任何語言，自動判斷）流暢地翻譯為 {target_language}。輸入格式為「[發送者] 訊息內容」。遵循以下規則：\n"
        "1. 只翻譯「訊息內容」，開頭的「[發送者]」原樣保留、不要翻譯或改動。\n"
        "2. 僅輸出「[發送者] 譯文」，禁止解釋或添加任何額外內容"
        "（如「以下是翻譯：」、「譯文如下：」等）。\n"
        "3. 忠實傳達原文的意思與語氣，不要曲解或改變原意。\n"
        f"4. 遊戲相關名詞（魔法名、地名、物品名、NPC 名等）翻成 {target_language}，並在譯名後"
        "用半形括號附上英文原文，例如「火龍(Fire Dragon)」、「鱷魚國(Krokotopia)」；"
        "純代碼或確實無法翻譯的內容則保留原文。\n"
        f"5. 網路及遊戲聊天的縮寫、俚語（如 lol、gg、brb、omg、ty、np 等）"
        f"請翻成 {target_language} 在地、口語的說法，不要保留原縮寫。\n"
        "6. 如果文本包含表情符號（emoji 或 :名稱: 形式），"
        "請原樣保留在對應位置，不要翻譯或刪除。\n"
        "7. 標點盡量貼近原文的標點風格（原文結尾沒有標點就盡量不加）；"
        f"需要標點時使用 {target_language} 慣用的樣式。"
    )


def build_outgoing_system(outgoing_language: str) -> str:
    """建構發話翻譯的 system 提示:把玩家輸入(任何語言,自動判斷)翻成 outgoing_language。"""
    return (
        f"你是一個專業的翻譯員，負責將玩家在線上遊戲 Wizard101 要發送的聊天訊息"
        f"（任何語言，自動判斷）流暢地翻譯為 {outgoing_language}。遵循以下規則：\n"
        "1. 僅輸出譯文，禁止解釋或添加任何額外內容"
        "（如「以下是翻譯：」、「譯文如下：」等）。\n"
        "2. 忠實傳達原文的意思與語氣，不要改寫、曲解、增添或省略內容"
        "（例如「我是台灣人」翻成「I'm Taiwanese」，而不是「I'm from Taiwan」）。\n"
        f"3. 用簡單、常見的 {outgoing_language} 字詞（遊戲聊天過濾器會擋掉罕見字），"
        "語氣口語自然、適合遊戲內聊天。\n"
        f"4. 遊戲相關名詞（魔法名、地名、物品名、NPC 名等）使用遊戲內慣用的 {outgoing_language} "
        "名稱，不要另譯或加註。\n"
        f"5. 網路及遊戲聊天的縮寫、俚語請翻成 {outgoing_language} 在地、口語的說法。\n"
        "6. 如果文本包含表情符號（emoji 或 :名稱: 形式），"
        "請原樣保留在對應位置，不要翻譯或刪除。\n"
        "7. 盡量貼近原文的標點風格（例如原文句尾沒有句號，譯文結尾也盡量不加）。"
    )


# thinking=False 時併入請求 body 的停用參數,涵蓋常見後端(伺服器通常忽略不認得的欄位)。
_DISABLE_THINKING = {
    "reasoning_effort": "none",                        # OpenAI o 系 / 相容
    "chat_template_kwargs": {"enable_thinking": False},  # vLLM / SGLang + Qwen3
    "think": False,                                    # Ollama
    "enable_thinking": False,                          # 部分伺服器吃頂層
}

# OpenAI 官方端點對未知欄位嚴格回 400，只能帶它自己認得的停用參數。
_DISABLE_THINKING_OPENAI = {"reasoning_effort": "none"}

_THINK_BLOCK = re.compile(r"<think>.*?</think>\s*", re.DOTALL | re.IGNORECASE)


def strip_think(text: str) -> str:
    """移除回應中的 <think>…</think> 推理區塊(reasoning 模型會把思考夾在 content 裡)。
    無論是否啟用思考都套用,確保推理內容不會污染譯文。"""
    return _THINK_BLOCK.sub("", text)


OPENAI_BASE_URL = "https://api.openai.com"  # ChatGPT preset 固定官方端點
_TIMEOUT = 60.0
_CLAUDE_MAX_TOKENS = 1024
TEST_SAMPLE = "[Tester] Hello! How are you?"  # 測試連線用固定原文


class TranslatorOffline(Exception):
    """可重試的翻譯失敗：連線失敗、逾時、429、5xx。"""


class TranslatorConfigError(Exception):
    """不可重試的設定錯誤：金鑰無效（401/403）、模型不存在（404）。"""

    def __init__(self, message: str, status: int | None = None):
        super().__init__(message)
        self.status = status


class _OpenAICompatClient:
    """OpenAI 相容端點（ChatGPT 官方與自訂伺服器共用）：打 /v1/chat/completions。"""

    def __init__(self, base_url: str, model: str, api_key: str = "",
                 thinking: bool = True, timeout: float = _TIMEOUT, client=None,
                 disable_params: dict = _DISABLE_THINKING):
        self._disable_params = disable_params
        if client is not None:
            self._client = client
        else:
            headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
            self._client = httpx.Client(base_url=base_url, headers=headers, timeout=timeout)
        self._model = model
        self._thinking = thinking

    def chat(self, system: str, text: str) -> str:
        body = {
            "model": self._model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
            "temperature": 0,
        }
        if not self._thinking:
            body.update(self._disable_params)
        try:
            resp = self._client.post("/v1/chat/completions", json=body)
        except httpx.HTTPError as exc:
            raise TranslatorOffline(str(exc)) from exc
        if resp.status_code in (401, 403, 404):
            raise TranslatorConfigError(f"HTTP {resp.status_code}", status=resp.status_code)
        if resp.status_code == 429 or resp.status_code >= 500:
            raise TranslatorOffline(f"HTTP {resp.status_code}")
        resp.raise_for_status()
        content = resp.json()["choices"][0]["message"]["content"]
        return strip_think(content).strip()


class _ClaudeClient:
    """Claude 官方 API（anthropic SDK）：打 /v1/messages。
    Claude 5 系不接受 temperature（會 400），thinking 用預設（adaptive），皆不帶。"""

    def __init__(self, model: str, api_key: str, timeout: float = _TIMEOUT, client=None):
        self._client = client if client is not None else anthropic.Anthropic(
            api_key=api_key, timeout=timeout)
        self._model = model

    def chat(self, system: str, text: str) -> str:
        try:
            resp = self._client.messages.create(
                model=self._model, max_tokens=_CLAUDE_MAX_TOKENS,
                system=system, messages=[{"role": "user", "content": text}])
        except anthropic.APIConnectionError as exc:
            raise TranslatorOffline(str(exc)) from exc
        except anthropic.APIStatusError as exc:
            code = exc.status_code
            if code in (401, 403, 404):
                raise TranslatorConfigError(f"HTTP {code}", status=code) from exc
            if code == 429 or code >= 500:
                raise TranslatorOffline(f"HTTP {code}") from exc
            raise
        content = "".join(b.text for b in resp.content if b.type == "text")
        return strip_think(content).strip()


def _build_client(provider: str, base_url: str, model: str, api_key: str,
                  thinking: bool, timeout: float, client):
    if provider == "claude":
        return _ClaudeClient(model=model, api_key=api_key, timeout=timeout, client=client)
    if provider == "openai":
        # 官方端點固定 base_url，且只帶它認得的停用參數（自架後端那組未知欄位會 400）。
        return _OpenAICompatClient(base_url=OPENAI_BASE_URL, model=model, api_key=api_key,
                                   thinking=thinking, timeout=timeout, client=client,
                                   disable_params=_DISABLE_THINKING_OPENAI)
    return _OpenAICompatClient(base_url=base_url, model=model, api_key=api_key,
                               thinking=thinking, timeout=timeout, client=client)


class Translator:
    """共用翻譯 client：依 provider 選擇後端，收訊/發話介面不變。"""

    def __init__(self, *, provider: str = "custom", base_url: str = "", model: str = "",
                 api_key: str = "", thinking: bool = True, target_language: str,
                 timeout: float = _TIMEOUT, client=None):
        self._impl = _build_client(provider, base_url, model, api_key, thinking,
                                   timeout, client)
        self._target_language = target_language

    def reconfigure(self, *, provider: str, base_url: str, model: str, api_key: str,
                    thinking: bool, target_language: str) -> None:
        """設定變更後就地重建後端 client（呼叫端不需換 Translator 實例）。"""
        self._impl = _build_client(provider, base_url, model, api_key, thinking,
                                   _TIMEOUT, None)
        self._target_language = target_language

    def translate_incoming(self, text: str) -> str:
        """收訊：把遊戲聊天（任何語言）翻成使用者設定的目標語言。"""
        return self._impl.chat(build_incoming_system(self._target_language), text)

    def translate_outgoing(self, text: str) -> str:
        """發話：把玩家輸入（任何語言）翻成遊戲聊天語言（固定）。"""
        return self._impl.chat(build_outgoing_system(OUTGOING_LANGUAGE), text)


def test_translate(api: dict, target_language: str) -> str:
    """測試連線：用表單當下的 api 設定實際翻一句固定文字，與正式翻譯同一條路。"""
    return Translator(**api, target_language=target_language).translate_incoming(TEST_SAMPLE)
