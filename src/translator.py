"""共用翻譯 client:打自架的 OpenAI 相容 /v1/chat/completions。"""
import httpx

ZH_SYSTEM = (
    "你是線上遊戲 Wizard101 的聊天翻譯員。把玩家的英文聊天訊息翻成自然、口語的"
    "繁體中文(台灣用語)。遊戲術語(咒語名、地名、物品名、Boss 名)保留英文原文。"
    "只輸出譯文,不要任何解釋或標點以外的附加內容。"
)

EN_SYSTEM = (
    "You translate a player's Traditional Chinese chat messages into casual, short "
    "English suitable for in-game chat in the MMO Wizard101. Use simple, common words "
    "(the game's chat filter blocks uncommon words). Keep game terms as-is. "
    "Output only the translation, nothing else."
)


class Translator:
    def __init__(self, base_url: str, model: str, api_key: str = "",
                 timeout: float = 10.0, client: httpx.Client | None = None):
        if client is not None:
            self._client = client
        else:
            headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
            self._client = httpx.Client(base_url=base_url, headers=headers, timeout=timeout)
        self._model = model

    def _chat(self, system: str, text: str) -> str:
        resp = self._client.post("/v1/chat/completions", json={
            "model": self._model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
            "temperature": 0.3,
        })
        resp.raise_for_status()
        return resp.json()["choices"][0]["message"]["content"].strip()

    def to_zh(self, text: str) -> str:
        return self._chat(ZH_SYSTEM, text)

    def to_en(self, text: str) -> str:
        return self._chat(EN_SYSTEM, text)
