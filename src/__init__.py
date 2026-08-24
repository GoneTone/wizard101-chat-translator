"""套件根：對外只提供版本號。

`__version__` 是版本的唯一真實來源。專案設定 `package = false`（不建置安裝），
打包成 exe 後也沒有 dist-info，因此 `importlib.metadata` 兩種情境都讀不到；
一個常數在原始碼執行與 frozen exe 下行為一致。`pyproject.toml` 的 version
僅是中繼資料，由 tests/test_version.py 釘住兩者一致。
"""
__version__ = "0.1.0"
