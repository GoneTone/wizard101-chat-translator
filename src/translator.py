"""共用翻譯 client:打自架的 OpenAI 相容 /v1/chat/completions。

收訊:來源語言自動判斷 → 翻成使用者設定的目標語言(target_language)。
發話:來源語言自動判斷 → 翻成遊戲聊天語言(OUTGOING_LANGUAGE,固定)。
提示詞依語言參數動態建構,程式碼不綁定特定語言。
"""
import re

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

_THINK_BLOCK = re.compile(r"<think>.*?</think>\s*", re.DOTALL | re.IGNORECASE)


def strip_think(text: str) -> str:
    """移除回應中的 <think>…</think> 推理區塊(reasoning 模型會把思考夾在 content 裡)。
    無論是否啟用思考都套用,確保推理內容不會污染譯文。"""
    return _THINK_BLOCK.sub("", text)


class Translator:
    def __init__(self, base_url: str, model: str, *, target_language: str,
                 api_key: str = "", thinking: bool = True,
                 timeout: float = 10.0, client: httpx.Client | None = None):
        if client is not None:
            self._client = client
        else:
            headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
            self._client = httpx.Client(base_url=base_url, headers=headers, timeout=timeout)
        self._model = model
        self._target_language = target_language
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

    def translate_incoming(self, text: str) -> str:
        """收訊:把遊戲聊天(任何語言)翻成使用者設定的目標語言。"""
        return self._chat(build_incoming_system(self._target_language), text)

    def translate_outgoing(self, text: str) -> str:
        """發話:把玩家輸入(任何語言)翻成遊戲聊天語言(固定)。"""
        return self._chat(build_outgoing_system(OUTGOING_LANGUAGE), text)
