# build.spec —— PyInstaller 打包設定:單一 exe。
# spike 階段 console=True 便於除錯;定稿(Task 7)改 console=False。
a = Analysis(
    ["run.py"],
    pathex=[],
    binaries=[],
    datas=[],
    hiddenimports=[],  # spike 發現缺模組時補在這裡,並註明原因
    excludedimports=[],
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    name="Wizard101ChatTranslator",
    console=True,
    upx=False,
)
