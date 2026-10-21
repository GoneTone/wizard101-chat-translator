# 發布版本

版本號遵循 [SemVer](https://semver.org/lang/zh-TW/)，唯一真實來源是 `Cargo.toml` 的 `version`（程式內讀 `env!("CARGO_PKG_VERSION")`）。破壞性變更（例如 `config.json` 欄位改名）一律進 major 版，重大更新也可能升 major；minor 版加功能、patch 版只修 bug。

發布版本由 GitHub Actions 的 `release-windows` workflow（`.github/workflows/release-windows.yml`）完成，不必在本機打包：

1. 到 GitHub 的 **Actions → release-windows → Run workflow**，填入 tag（`v0.2.0`，一律 `v` 前綴；預發版寫 `v0.2.0-rc.1` 並勾 pre-release）。
2. workflow 會在 `master` 上（clippy 與測試交給 `master` 的 CI 把關，這裡不跑；發版前先確認最新一輪 CI 是綠的）：把 `Cargo.toml` 與 `Cargo.lock` 的版本號改成 tag 的版本 → 跑 fmt → `cargo build --release --locked` 打包 → 用 `.github/scripts/verify-exe.ps1` 檢查 exe 的版本、圖示與 manifest 資源 → 都通過才 commit 並推上 `master`（`chore(release): bump version to v0.2.0 [skip ci]`；版本號沒變就不 commit；任一步失敗則什麼都不會推）→ 以該 commit 建立**草稿** Release、上傳 `Wizard101ChatTranslator.exe`，版本說明用 GitHub 自動產生的 What's Changed 加上 `.github/release-footer.md` 的固定文案。
3. 下載草稿裡的 exe，用 Windows Defender 掃描（PowerShell 執行 `& "$env:ProgramFiles\Windows Defender\MpCmdRun.exe" -Scan -ScanType 3 -File <exe>`，或在檔案總管對 exe 按右鍵掃描）。被誤判就到 Microsoft 的[誤判提交頁](https://www.microsoft.com/en-us/wdsi/filesubmission)送審，通過後再發佈。
4. 到 Releases 頁檢查草稿（可在最上方補一段人寫的版本說明），確認後按 **Publish**。

> 更新檢查看的是 GitHub 的 **Release**（`/releases/latest`），草稿與 pre-release 都不算，要按下 Publish 使用者端才會收到更新提醒。同一個 tag 重跑 workflow：草稿還在的話，只會把它改指向新的 commit 並重傳 exe，版本說明（含你手寫的那段）維持原樣、不會重新產生；若該 tag 的 Release 已經發布，workflow 會直接失敗並拒絕覆蓋它的檔案。
