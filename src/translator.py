"""共用翻譯 client:打自架的 OpenAI 相容 /v1/chat/completions。"""
import re

import httpx

ZH_SYSTEM = (
    "你是一個專業的繁體中文母語譯者，負責將線上遊戲 Wizard101 的聊天對話文本"
    "流暢地翻譯為繁體中文（台灣）。輸入格式為「[發送者] 訊息內容」。遵循以下規則：\n"
    "1. 只翻譯「訊息內容」，開頭的「[發送者]」原樣保留、不要翻譯或改動。\n"
    "2. 僅輸出「[發送者] 譯文」，禁止解釋或添加任何額外內容"
    "（如「以下是翻譯：」、「譯文如下：」等）。\n"
    "3. 遊戲相關名詞（咒語名、地名、物品名、NPC 名等）翻成中文，並在譯名後用半形括號"
    "附上英文原文，例如「火龍(Fire Dragon)」、「鱷魚國(Krokotopia)」；"
    "純代碼或確實無法翻譯的內容則保留原文。\n"
    "4. 網路及遊戲聊天的縮寫、俚語（如 lol、gg、brb、omg、ty、np 等）請翻成台灣在地、"
    "口語的說法（例如 lol→笑死、brb→馬上回來、ty→謝啦、np→不會），不要保留原縮寫。\n"
    "5. 如果文本包含表情符號（emoji 或 :名稱: 形式），"
    "請原樣保留在對應位置，不要翻譯或刪除。\n"
    "6. 標點符號使用台灣慣用的全形標點（，。？！：；、「」……）。"
)

# 中文語境判定:CJK 統一表意文字、CJK 標點(、。「」等)、全形標點
_CJK = "　-〿一-鿿！-～…"
_HALF_TO_FULL = {",": "，", "!": "！", "?": "？", ";": "；", ":": "："}


def normalize_zh_punct(text: str) -> str:
    """把「緊跟在中文之後」的半形標點轉成全形（台灣慣例）。
    模型不一定每次都用全形，故輸出後再做一次確定性修正。
    只在中文語境轉換，不動英文片段、數字（1,000）、表情符號（:)）與網址。"""
    out = re.sub(rf"(?<=[{_CJK}])\.{{3,}}", "……", text)
    out = re.sub(rf"(?<=[{_CJK}])\.(?=$|\s|[{_CJK}])", "。", out)
    for half, full in _HALF_TO_FULL.items():
        out = re.sub(rf"(?<=[{_CJK}]){re.escape(half)}", full, out)
    out = re.sub(rf"\((?=[{_CJK}])", "（", out)
    out = re.sub(rf"(?<=[{_CJK}])\)", "）", out)
    return out


EN_SYSTEM = (
    "You translate a player's Traditional Chinese chat messages into casual, short "
    "English suitable for in-game chat in the MMO Wizard101. Use simple, common words "
    "(the game's chat filter blocks uncommon words). Keep game terms as-is. "
    "Output only the translation, nothing else."
)

# thinking=False 時併入請求 body 的停用參數,涵蓋常見後端(伺服器通常忽略不認得的欄位)。
_DISABLE_THINKING = {
    "reasoning_effort": "none",                        # OpenAI o 系 / 相容
    "chat_template_kwargs": {"enable_thinking": False},  # vLLM / SGLang + Qwen3
    "think": False,                                    # Ollama
    "enable_thinking": False,                          # 部分伺服器吃頂層
}

_THINK_BLOCK = re.compile(r"<think>.*?</think>\s*", re.DOTALL | re.IGNORECASE)


def strip_think(text: str) -> str:
    """移除回應中的 <think>…</think> 推理區塊(reasoning 模型會把思考夾在 content 裡)。
    無論是否啟用思考都套用,確保推理內容不會污染譯文。"""
    return _THINK_BLOCK.sub("", text)


class Translator:
    def __init__(self, base_url: str, model: str, api_key: str = "", thinking: bool = True,
                 timeout: float = 10.0, client: httpx.Client | None = None):
        if client is not None:
            self._client = client
        else:
            headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
            self._client = httpx.Client(base_url=base_url, headers=headers, timeout=timeout)
        self._model = model
        self._thinking = thinking

    def _chat(self, system: str, text: str) -> str:
        body = {
            "model": self._model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
            "temperature": 0.3,
        }
        if not self._thinking:
            body.update(_DISABLE_THINKING)  # 預設 thinking=True:body 不帶任何思考相關參數
        resp = self._client.post("/v1/chat/completions", json=body)
        resp.raise_for_status()
        content = resp.json()["choices"][0]["message"]["content"]
        return strip_think(content).strip()

    def to_zh(self, text: str) -> str:
        return normalize_zh_punct(self._chat(ZH_SYSTEM, text))

    def to_en(self, text: str) -> str:
        return self._chat(EN_SYSTEM, text)
