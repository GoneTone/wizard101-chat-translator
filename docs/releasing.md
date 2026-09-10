# 發布版本

版本號遵循 [SemVer](https://semver.org/lang/zh-TW/)，唯一真實來源是 `src/__init__.py` 的 `__version__`（`pyproject.toml` 的 `version` 只是中繼資料，由 `tests/test_version.py` 釘住兩者一致）。破壞性變更（例如 `config.json` 欄位改名）一律進 major 版，重大更新也可能升 major；minor 版加功能、patch 版只修 bug。

發布版本由 GitHub Actions 的 `release-windows` workflow（`.github/workflows/release-windows.yml`）完成，不必在本機打包：

1. 到 GitHub 的 **Actions → release-windows → Run workflow**，填入 tag（`v0.2.0`，一律 `v` 前綴；預發版寫 `v0.2.0-rc.1` 並勾 pre-release）。
2. workflow 會在 `master` 上：把 `src/__init__.py`、`pyproject.toml`、`uv.lock` 的版本號改成 tag 的版本並 commit（`chore(release): bump version to v0.2.0 [skip ci]`）→ 跑 lint 與測試 → `pyinstaller` 打包 → 以該 commit 建立**草稿** Release、上傳 `Wizard101ChatTranslator.exe`，版本說明用 GitHub 自動產生的 What's Changed 加上 `.github/release-footer.md` 的固定文案。
3. 到 Releases 頁檢查草稿（可在最上方補一段人寫的版本說明），確認後按 **Publish**。

> 更新檢查看的是 GitHub 的 **Release**（`/releases/latest`），草稿與 pre-release 都不算，要按下 Publish 使用者端才會收到更新提醒。同一個 tag 重跑 workflow：草稿還在的話，只會把它改指向新的 commit 並重傳 exe，版本說明（含你手寫的那段）維持原樣、不會重新產生；若該 tag 的 Release 已經發布，workflow 會直接失敗並拒絕覆蓋它的檔案。
