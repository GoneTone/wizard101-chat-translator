# 檢查建出的 exe 帶有版本資訊、圖示與 per-monitor DPI manifest；不符就丟例外讓 workflow 失敗。
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Version
)

$ErrorActionPreference = 'Stop'
$file = Get-Item $Exe

$info = $file.VersionInfo
$expected = [ordered]@{
    ProductName      = 'Wizard101 Chat Translator'
    FileDescription  = 'Wizard101 Chat Translator'
    OriginalFilename = 'Wizard101ChatTranslator.exe'
    ProductVersion   = $Version
}
foreach ($key in $expected.Keys) {
    $actual = $info.$key
    Write-Host "$key = '$actual' (expected '$($expected[$key])')"
    if ($actual -ne $expected[$key]) { throw "exe $key mismatch: '$actual' vs '$($expected[$key])'" }
}

Add-Type -Namespace Native -Name Res -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
public static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern uint SizeofResource(IntPtr module, IntPtr resource);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
[DllImport("kernel32.dll")]
public static extern IntPtr LockResource(IntPtr data);
[DllImport("kernel32.dll")]
public static extern bool FreeLibrary(IntPtr module);
'@

# LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE：只讀資源，不執行 exe
$module = [Native.Res]::LoadLibraryExW($file.FullName, [IntPtr]::Zero, 0x22)
if ($module -eq [IntPtr]::Zero) { throw "cannot load $($file.FullName) as a data file" }
try {
    $RT_GROUP_ICON = [IntPtr]14
    $RT_MANIFEST = [IntPtr]24
    if ([Native.Res]::FindResourceW($module, [IntPtr]1, $RT_GROUP_ICON) -eq [IntPtr]::Zero) {
        throw "exe has no icon resource"
    }
    $manifest = [Native.Res]::FindResourceW($module, [IntPtr]1, $RT_MANIFEST)
    if ($manifest -eq [IntPtr]::Zero) { throw "exe has no manifest resource" }
    $size = [Native.Res]::SizeofResource($module, $manifest)
    $bytes = New-Object byte[] $size
    [Runtime.InteropServices.Marshal]::Copy([Native.Res]::LockResource([Native.Res]::LoadResource($module, $manifest)), $bytes, 0, $size)
    if (-not [Text.Encoding]::UTF8.GetString($bytes).Contains('PerMonitorV2')) {
        throw "exe manifest lacks PerMonitorV2"
    }
} finally {
    [void][Native.Res]::FreeLibrary($module)
}
Write-Host "exe resources OK (version $Version)"
