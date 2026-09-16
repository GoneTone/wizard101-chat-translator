"""啟動畫面底圖：build 時生成，不進版控。

PyInstaller 的 Splash 只提供一行動態文字（單一 `text_pos`），所以名稱與版本號必須先
畫進底圖。版本號從 `src.__version__` 帶入，維持版本號的單一真實來源 —— 每次發布都會
變，不該有第二份要手改的圖。

狀態文字由 Tcl 以 `-anchor sw`（左下）畫上去、且長度會變，沒有辦法置中，所以這裡把它
的位置讓給左下角，icon 與標題則置中 —— 排版是配合那個限制設計的，不是沒對齊。
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from src import __version__
from src.log import log
from src.resources import icon_path

SIZE = (480, 300)
TEXT_ORIGIN = (24, 276)      # 狀態文字的左下錨點，build.spec 的 text_pos 要用同一組值

_TITLE = "Wizard101 Chat Translator"
# 底色避開 magenta（#ff00ff）—— Windows 版的 splash 拿它當透明色（見 PyInstaller 的
# transparent_setup 模板），用到的話整塊會被挖成透明。
_BACKGROUND = "#1b1b2f"
_TITLE_COLOR = "#f0f0f5"
# 狀態文字沿用標題色（對比色取自 _BACKGROUND），build.spec 的 text_color 要用同一組值
TEXT_COLOR = _TITLE_COLOR
_VERSION_COLOR = "#9a9ab0"
_ICON_SIZE = 128             # icon.ico 的原生 frame，不放大
_ICON_TOP = 40
_TITLE_TOP = 186
_VERSION_TOP = 220
# Windows 的 Segoe UI Semibold 檔名是 seguisb.ttf（不是 segoeuisb.ttf）。找不到就退
# 一般體、再退 Pillow 內建 —— 字型只影響美觀，不值得讓 release 掛掉。
_TITLE_FONTS = ("seguisb.ttf", "segoeui.ttf")
_VERSION_FONTS = ("segoeui.ttf",)


def _font(candidates: tuple[str, ...], size: int):
    for name in candidates:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    log(f"[build] none of {candidates} is installed; splash falls back to the default font")
    return ImageFont.load_default(size)


def _centered(draw: ImageDraw.ImageDraw, top: int, text: str, font, color: str) -> None:
    width = draw.textbbox((0, 0), text, font=font)[2]
    draw.text(((SIZE[0] - width) // 2, top), text, font=font, fill=color)


def build_splash_image(dest: Path) -> Path:
    """畫出底圖並存成 PNG，回傳實際寫出的路徑。

    失敗一律往上拋，讓 build 紅掉 —— 默默發出一個沒有啟動畫面的版本，比 build 失敗糟。
    """
    canvas = Image.new("RGB", SIZE, _BACKGROUND)
    icon = Image.open(icon_path())
    icon.size = (_ICON_SIZE, _ICON_SIZE)   # ICO 有多個 frame，指定要取的原生尺寸
    icon = icon.convert("RGBA")
    canvas.paste(icon, ((SIZE[0] - _ICON_SIZE) // 2, _ICON_TOP), icon)

    draw = ImageDraw.Draw(canvas)
    _centered(draw, _TITLE_TOP, _TITLE, _font(_TITLE_FONTS, 22), _TITLE_COLOR)
    _centered(draw, _VERSION_TOP, f"v{__version__}", _font(_VERSION_FONTS, 14),
              _VERSION_COLOR)

    dest.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(dest, "PNG")
    log(f"[build] splash image written: path={dest} size={SIZE} version={__version__}")
    return dest
