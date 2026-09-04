"""診斷輸出：一行訊息寫到 stderr（打包版已導向 exe 旁的 app.log）。

訊息一律英文、帶 `[模組]` 前綴（`[reader]`／`[translate]`／`[ui]`…），內容要有足夠
context（行數、狀態碼、PID、例外訊息）讓使用者匯出 app.log 就能定位問題；
API 金鑰之類的敏感資料絕不可寫入。時戳由 main 掛上的 TimestampedStream 補。"""
import sys


def log(message: str) -> None:
    stream = sys.stderr
    if stream is None:   # windowed exe 在 main 把輸出導向 app.log 之前 stderr 是 None
        return
    # 整行一次 write（不用 print：它把內文與換行分兩次寫），多執行緒同時記錄時
    # 每一行才不會被別人的行切開、或漏掉行首時戳
    stream.write(message + "\n")
