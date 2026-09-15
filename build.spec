# build.spec —— PyInstaller 打包設定：單一 exe。
# 定稿：console=False（windowed，無黑窗）；執行期輸出改導向 exe 旁的 app.log
# （見 src/main.py 的 frozen 判斷），使用者回報問題附上該檔即可。
import sys

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

a = Analysis(
    ["run.py"],
    pathex=[],
    binaries=[],
    datas=[
        ("src/i18n/*.json", "i18n"),      # 語言檔：執行期由 src/i18n 依 _MEIPASS 讀取
        ("src/assets/*", "assets"),       # icon：exe 用 .ico，tkinter 用 .png，都經 src/resources 取用
    ],
    # pywinrt 的集合與語言投影是 WinRT 回傳物件時執行期動態載入，靜態分析追不到；
    # 缺了打包版的本機 OCR 會在列舉辨識語言時 ModuleNotFoundError（見 src/region/ocr.py）
    hiddenimports=[
        "winrt.windows.foundation",
        "winrt.windows.foundation.collections",
        "winrt.windows.globalization",
    ],
    excludes=[],
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    name="Wizard101ChatTranslator",
    icon="src/assets/icon.ico",
    version=VERSION_INFO,
    console=False,
    upx=False,
)
