# build.spec —— PyInstaller 打包設定：單一 exe。
# 定稿：console=False（windowed，無黑窗）；執行期輸出改導向 exe 旁的 app.log
# （見 src/main.py 的 frozen 判斷），使用者回報問題附上該檔即可。
import sys
from pathlib import Path

from PyInstaller.utils.hooks import collect_data_files
from PyInstaller.utils.win32.versioninfo import (
    FixedFileInfo,
    StringFileInfo,
    StringStruct,
    StringTable,
    VarFileInfo,
    VarStruct,
    VSVersionInfo,
)

sys.path.insert(0, SPECPATH)   # SPECPATH 由 PyInstaller 注入 spec 的命名空間
from src import __version__    # 版本號的唯一真實來源，不在這裡另抄一份
from tools.splash_image import TEXT_COLOR, TEXT_ORIGIN, build_splash_image
from tools.splash_progress import install_progress_bar

# 檔案版本欄位只吃四個數字，預發布版（0.2.0-rc.1）的後綴只留在字串欄位。
_numbers = tuple(int(n) for n in __version__.split("-")[0].split("."))
FILE_VERSION = (_numbers + (0, 0, 0, 0))[:4]

# 只宣告一組語言（en-US）：Windows 的多語系版本資訊是「每個語言各一份 RT_VERSION
# 資源」，PyInstaller 寫的是單一語言中性資源，同一份資源裡放多張 StringTable 不會依
# 使用者語言切換 —— 實測檔案總管一律取第一張。故一律用英文，各語系系統都不會缺字。
VERSION_INFO = VSVersionInfo(
    ffi=FixedFileInfo(filevers=FILE_VERSION, prodvers=FILE_VERSION),
    kids=[
        StringFileInfo([StringTable("040904b0", [
            StringStruct("CompanyName", "GoneTone"),
            # 工作管理員的「名稱」欄顯示 FileDescription，放短名稱、完整介紹放 Comments
            StringStruct("FileDescription", "Wizard101 Chat Translator"),
            StringStruct("FileVersion", __version__),
            StringStruct("InternalName", "Wizard101ChatTranslator"),
            StringStruct("LegalCopyright", "© 2026 GoneTone"),
            StringStruct("OriginalFilename", "Wizard101ChatTranslator.exe"),
            StringStruct("ProductName", "Wizard101 Chat Translator"),
            StringStruct("ProductVersion", __version__),
            StringStruct("Comments",
                         "Wizard101 Chat Translator — an AI translator for "
                         "Wizard101's in-game chat, translating conversations "
                         "both ways in real time."),
        ])]),
        VarFileInfo([VarStruct("Translation", [1033, 1200])]),
    ],
)

# 啟動畫面的底圖：每次 build 重新生成，版本號才不會是舊的（見 tools/splash_image.py）
SPLASH_IMAGE = build_splash_image(Path(SPECPATH) / "build" / "splash.png")

a = Analysis(
    ["run.py"],
    pathex=[],
    binaries=[],
    datas=[
        ("src/i18n/*.json", "i18n"),      # 語言檔：執行期由 src/i18n 依 _MEIPASS 讀取
        ("src/assets/*", "assets"),       # icon：exe 用 .ico，tkinter 用 .png，都經 src/resources 取用
        # 本機 OCR（rapidocr）的模型與設定 yaml 都放在套件目錄裡、執行期用相對路徑讀取，
        # 靜態分析不會把這些非 .py 檔一起打包；缺了會在建立引擎時報找不到
        # default_models.yaml（見 src/region/ocr.py）
        *collect_data_files("rapidocr"),
    ],
    hiddenimports=[],
    excludes=[
        # AVIF 解碼器（PIL/_avif.pyd，7.9 MB 未壓縮）：本專案只處理 PNG 與螢幕擷取。
        # 切斷 import 鏈通常 .pyd 就不會被收 —— 打包後要用 Step 4 確認它真的不在。
        "PIL.AvifImagePlugin",
    ],
)
# cv2 的 ffmpeg 解碼器（30.9 MB 未壓縮）：rapidocr 只用影像處理 API，不碰 VideoCapture。
# 它不是 Python 模組，`excludes` 管不到，而 `exclude_system_libraries()` 只對 POSIX
# 有效（只掃 /lib*、/usr/lib*），所以只能在 Analysis 之後從 binaries 濾掉。
a.binaries = [b for b in a.binaries if "opencv_videoio_ffmpeg" not in b[0]]
# 進度條依位元組數加權，不是檔案數：少數大檔案佔掉大半體積，見
# docs/superpowers/specs/2026-09-16-faster-exe-startup-design.md。binaries 與 datas
# 都要給——只算 binaries 會漏掉 rapidocr 的模型檔（a.datas）。必須排在 ffmpeg 過濾
# 之後、Splash(...) 建構之前 —— 過濾前算會把已排除的檔案也算進總數，Splash 建構後
# 再改樣板已經來不及（見 tools/splash_progress.py 的模組說明）。
install_progress_bar(a.binaries, a.datas)
splash = Splash(
    str(SPLASH_IMAGE),
    binaries=a.binaries,   # 讓它偵測到 tkinter 已經打包了 tcl/tk，沿用而不再塞一份
    datas=a.datas,
    text_pos=TEXT_ORIGIN,
    text_size=10,
    text_color=TEXT_COLOR,
    # 解壓期間顯示的字：打包時就寫死，那時 Python 還沒啟動、讀不到介面語言設定。
    # 一律 ASCII —— 這串字會被寫進 bootloader 的 Tcl 腳本。
    text_default="Initializing...",
    always_on_top=True,
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    splash,
    splash.binaries,
    a.binaries,
    a.datas,
    name="Wizard101ChatTranslator",
    icon="src/assets/icon.ico",
    version=VERSION_INFO,
    console=False,
    upx=False,
)
