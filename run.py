"""啟動器：`uv run run.py`（等同 `python -m src.main`）。"""


def _run() -> None:
    """先把啟動畫面的文字換掉再 import —— `src.main` 的 import 要約 0.3 秒，
    這段時間畫面上還停在 bootloader 寫死的 `Initializing...`。

    兩個 import 都放在函式內：模組頂層的 import 會被 ruff 的 E402 擋下，而順序
    （先 update 再 import src.main）正是這裡的重點。
    """
    from src import splash
    splash.update("Loading components...")
    from src.main import main
    main()


if __name__ == "__main__":
    _run()
