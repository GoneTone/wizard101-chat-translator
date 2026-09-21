"""套件根：對外只提供版本號。

`__version__` 是版本的唯一真實來源：專案 `package = false`、exe 也沒有 dist-info，
`importlib.metadata` 兩種情境都讀不到。`pyproject.toml` 的 version 只是中繼資料，
由 tests/test_version.py 釘住兩者一致。
"""
__version__ = "0.4.0"
