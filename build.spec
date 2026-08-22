# build.spec —— PyInstaller 打包設定：單一 exe。
# 定稿：console=False（windowed，無黑窗）；執行期輸出改導向 exe 旁的 app.log
# （見 src/main.py 的 frozen 判斷），使用者回報問題附上該檔即可。
a = Analysis(
    ["run.py"],
    pathex=[],
    binaries=[],
    datas=[],
    hiddenimports=[],  # spike 發現缺模組時補在這裡，並註明原因
    excludes=[],
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    name="Wizard101ChatTranslator",
    console=False,
    upx=False,
)
