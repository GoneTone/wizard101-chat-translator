# 遊戲 2026-11-12 改版：更新三個 hook

## 目的

遊戲 2026-11-12 更新 `WizardGraphicalClient.exe`（整份重新編譯，暫存器分配與函式開頭都變了），`app.log` 顯示 autobot pattern 比對失敗、收訊整個掛不上。讓翻譯器重新掛得上新版。

## 目標／非目標／反目標

- 目標：autobot cave、AddLine、root window 三個 hook 在新版各自唯一命中並正確運作。
- 非目標：同時支援舊版（登入時強制更新，舊版客戶端進不了遊戲）；發版（使用者自己做）。
- 反目標：pattern 比對得到卻掛錯位置或暫存器，讓遊戲當掉。

## 發現（對新版 exe 靜態分析）

| hook | 新版狀況 | 處理 |
|---|---|---|
| autobot cave | 舊 pattern 0 命中 | 換新 pattern（Deimos wizwalker PR #36；StarrFox 原版 pattern 也唯一命中同一函式 `0x14170cb10`）。函式 3975 bytes、下一個函式在 +3984，`AUTOBOT_SIZE` 由 4100 降為 3900 |
| AddLine | 舊 pattern 0 命中 | 新入口 `0x14073fa80`；rcx＝this、rdx＝wstring、r8d＝分頁、`cmp tab,5`、wstring 佈局（size +0x10、capacity +0x18、SSO < 8）都與舊版相同。開頭變成 `push rbp; push rsi; push rdi; push r12`（`40 55 56 57 41 54`，6 bytes） |
| root window | 舊 pattern 仍命中，但指令改為 `mov rcx,[r15+D8]` | stub 照舊用 r13 會讓遊戲當掉；stub 改 r15，pattern 把暫存器寫死（採 PR #36 版本） |

ChatBox 偏移（`+0x7a0` 目前分頁、`+0x7a8`～`+0x848` 分頁緩衝）在新版的切分頁函式與 AddLine 內確認未變。

## 方案

1. **直接換成新版的 pattern／stub／cave 大小**（採用）：沿用 CLAUDE.md 記載的改版處理流程，改動最小。
2. stub 從比對到的位元組動態推導暫存器：程式較多，下次重新編譯可能連指令形狀都變，換不到多少穩健度。
3. 新舊兩組 pattern 並存：舊版已無法登入，屬 YAGNI。

## 設計

- `AUTOBOT_PATTERN` 換成 PR #36 版本，`AUTOBOT_SIZE = 3900`；保存的原始位元組數仍是 pattern 字串長度（wizwalker 慣例），相關測試的長度斷言跟著改。
- `ROOT_WINDOW_PATTERN` 換成寫死暫存器的版本；stub 的兩條指令改為 `49 8B 87 D8 00 00 00`（`mov rax,[r15+D8]`）與 `49 8B 8F D8 00 00 00`（原指令）。
- AddLine：跳轉點不放在入口（入口 6 bytes 是 4 條 push，跳轉會一次蓋過多條指令，掛入瞬間若有執行緒停在其間會當掉），改放在 7 個 push 之後、8 bytes 的 `lea rbp,[rsp-130]`（`48 8D AC 24 D0 FE FF FF`），只蓋這一條；新 pattern 從這條指令開始，位元組寫死。`instruction_length: 8`、`noops: 3`。stub 開頭照舊先執行原指令；讀回傳位址的偏移由 `[rsp+38]` 改為 `[rsp+70]`（函式的 7 個 push 加 stub 的 7 個），其餘位元組照舊。rcx／rdx／r8 在此點仍是原參數。
- 一併更新：`tests/common/fake_game`（`place_cave`／`place_root_window`／`place_addline`）、`tests/golden/hook_stubs.json`（已無 Python 版可重產，依新 stub 手動推導）、過時的註解（「wizwalker 原文」、「與 Python 版逐一相同」、「5 bytes」、「208」、「4100」）。

## 驗證

- 本機：`cargo test` 等提交前檢查；以 capstone 反組譯產生的 stub，對照新版 exe 確認搬移的位元組與暫存器。
- 實機（使用者執行）：登入進世界內 `cargo run`，確認三個 hook 掛上、收訊正常、分頁名稱讀得到（`LABEL_OFFSET` 0x2E0 與視窗樹偏移只推斷未變，PR #36 也沒動 window 偏移）。

## 留給實作者

- AddLine pattern 的萬用字元範圍：要保證新版唯一命中，並且固定 stub 搬移的開頭 6 bytes。
