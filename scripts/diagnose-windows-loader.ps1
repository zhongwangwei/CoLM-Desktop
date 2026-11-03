param([Parameter(Mandatory = $true)][string]$Executable)
$ErrorActionPreference = 'Stop'
$Executable = (Resolve-Path $Executable).Path
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$visualStudio = & $vswhere -latest -products '*' -property installationPath
$dumpbin = Get-ChildItem "$visualStudio\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" |
    Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
if (-not $dumpbin) { throw 'Visual Studio dumpbin was not found' }
$imports = & $dumpbin /nologo /imports $Executable
if ($LASTEXITCODE -ne 0) { throw "dumpbin failed: $LASTEXITCODE" }
$imports | Set-Content "$env:RUNNER_TEMP\windows-gui-imports.txt"
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class LoaderProbe {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibraryW(string path);
    [DllImport("kernel32.dll", CharSet = CharSet.Ansi, ExactSpelling = true, SetLastError = true)]
    public static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool SetDllDirectoryW(string path);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    public static extern uint GetModuleFileNameW(IntPtr module, StringBuilder path, int size);
    [DllImport("kernel32.dll")]
    public static extern bool FreeLibrary(IntPtr module);
}
'@
# Match the test executable's application directory during ordinary DLL resolution.
if (-not [LoaderProbe]::SetDllDirectoryW((Split-Path $Executable))) {
    throw 'Cannot set the test executable DLL directory'
}
$module = [IntPtr]::Zero
$library = ''
try {
    foreach ($line in $imports) {
        if ($line -match '^\s+(\S+\.dll)\s*$') {
            if ($module -ne [IntPtr]::Zero) { [void][LoaderProbe]::FreeLibrary($module) }
            $library = $Matches[1]
            $module = [LoaderProbe]::LoadLibraryW($library)
            $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            $resolved = [Text.StringBuilder]::new(32768)
            if ($module -ne [IntPtr]::Zero) {
                [void][LoaderProbe]::GetModuleFileNameW($module, $resolved, $resolved.Capacity)
            }
            Write-Output "DLL $library handle=$module error=$errorCode path=$resolved"
        } elseif ($module -ne [IntPtr]::Zero -and $line -match '^\s+[0-9A-Fa-f]+\s+(\w+)\s*$') {
            $symbol = $Matches[1]
            if ([LoaderProbe]::GetProcAddress($module, $symbol) -eq [IntPtr]::Zero) {
                Write-Output "MISSING ENTRY: $library!$symbol"
            }
        }
    }
} finally {
    if ($module -ne [IntPtr]::Zero) { [void][LoaderProbe]::FreeLibrary($module) }
    [void][LoaderProbe]::SetDllDirectoryW($null)
}
