"""翻譯失敗的分類：可重試（TranslatorOffline）、等使用者修設定（TranslatorConfigError）、
譯文被截斷（TranslatorBadOutput）、使用者取消（TranslatorCancelled）、端點不支援模型清單
（TranslatorNoModelList）。"""


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


class TranslatorConfigError(TranslatorError):
    """設定錯誤：4xx（金鑰無效、模型不存在、參數不被接受等）。
    可重試 —— pool 以固定的 CONFIG_ERROR_INTERVAL 間隔持續重試，
    使用者於執行期間修正 config.json 後即自動恢復，不必重啟程式。"""


class TranslatorCancelled(Exception):
    """請求被呼叫端取消（見 RequestHandle）：不是錯誤，呼叫端靜默丟掉即可。"""


class TranslatorNoModelList(Exception):
    """此端點不提供模型清單（/v1/models 回 404／405，或回應缺 data 陣列）。
    與 TranslatorConfigError 的 404（模型不存在）是兩回事：這裡只代表「問不到清單」，
    使用者仍可自行輸入模型名稱正常翻譯。"""
