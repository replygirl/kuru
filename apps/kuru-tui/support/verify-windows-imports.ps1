# Build-time acceptance only. The installed application never invokes MSVC tools.
[CmdletBinding()]
param([string]$Binary = $env:KURU_EMBEDDED_TEST_BINARY)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'PE import verification requires native Windows' }
if ([string]::IsNullOrWhiteSpace($Binary)) { throw 'Set KURU_EMBEDDED_TEST_BINARY to the shipping executable' }
# Native tools need the filesystem path, including any genuine extended prefix,
# rather than PowerShell's provider-qualified representation.
$Binary = (Resolve-Path -LiteralPath $Binary).ProviderPath
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) { throw 'MSVC Build Tools with vswhere are required for the shipping import check' }
$tools = @(& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe')
if ($LASTEXITCODE -ne 0 -or $tools.Count -eq 0) { throw 'The native MSVC dumpbin tool is missing' }
$dumpbin = $tools[-1]

# Provision from the verified embedded archive through its owning package.
# Do not accept an unrelated Dolt installation found on PATH.
$engineOutput = @(& mise run //packages/kuru-memory:prefetch)
if ($LASTEXITCODE -ne 0 -or $engineOutput.Count -eq 0) { throw 'Preparing the embedded Dolt for PE inspection failed' }
$engine = (Resolve-Path -LiteralPath $engineOutput[-1]).ProviderPath

# Windows 10/11 system DLLs and API-set contracts. In particular, accepting any
# DLL found on a developer machine would hide accidental VC redistributables.
$systemLibraries = @(
    'advapi32.dll', 'bcrypt.dll', 'bcryptprimitives.dll', 'combase.dll',
    'crypt32.dll', 'dbgcore.dll', 'dbghelp.dll', 'dnsapi.dll', 'iphlpapi.dll',
    'kernel32.dll', 'msvcrt.dll', 'ncrypt.dll', 'ntdll.dll', 'ole32.dll',
    'oleaut32.dll', 'rpcrt4.dll', 'secur32.dll', 'shell32.dll', 'shlwapi.dll',
    'ucrtbase.dll', 'user32.dll', 'userenv.dll', 'version.dll', 'winhttp.dll',
    'ws2_32.dll'
)
foreach ($image in @($Binary, $engine)) {
    # /IMPORTS includes delay-loaded DLLs as well as the ordinary import table.
    $imports = @(& $dumpbin /NOLOGO /IMPORTS $image)
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect PE imports: $image" }
    $libraries = @($imports | ForEach-Object {
        if ($_ -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
    } | Sort-Object -Unique)
    if ($libraries.Count -eq 0) {
        $diagnostic = ($imports | Select-Object -First 12) -join [Environment]::NewLine
        $diagnostic = $diagnostic.Substring(0, [Math]::Min(4096, $diagnostic.Length))
        throw "No DLL imports were parsed from ${image}: $diagnostic"
    }
    foreach ($library in $libraries) {
        if ($systemLibraries -notcontains $library -and $library -notmatch '^(api|ext)-ms-win-[a-z0-9-]+-l[0-9]+-[0-9]+-[0-9]+\.dll$') {
            throw "Shipping image requires an unapproved external DLL: $image -> $library"
        }
    }
    Write-Output ("OS-only PE imports: {0}: {1}" -f $image, ($libraries -join ', '))
}
