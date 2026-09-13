#requires -Version 5.1
[CmdletBinding()]
param(
    [string]$InputDir = $env:KURU_WINDOWS_DEBUGGER_INPUT_DIR
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$installerUrl = 'https://download.microsoft.com/download/4c09a46e-b908-42d9-bf27-26cb1779c670/KIT_BUNDLE_WINDOWSSDK_MEDIACREATION/winsdksetup.exe'
$installerName = 'winsdksetup-10.0.26100.9169.exe'
$installerSize = 1449536
$installerHash = '680aa29dcfa806d35b4e93ea05a3fa2bdcf2935b65295f996c8e944584dd2836'

function Require([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Require-RegularFile([string]$Path, [string]$Label) {
    $item = Get-Item -Force -LiteralPath $Path
    Require (-not $item.PSIsContainer) "$Label must be a regular file"
    Require (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) "$Label must not be a reparse point"
    return $item
}

function Require-MicrosoftSignature([string]$Path, [string]$Label) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    Require ($signature.Status -eq 'Valid') "$Label must have a valid Authenticode signature"
    $certificate = $signature.SignerCertificate
    Require ($null -ne $certificate) "$Label must include a signer certificate"
    Require ($certificate.Subject -match '(^|,\s*)O=Microsoft Corporation(,|$)') "$Label signer organization must be Microsoft Corporation"
    Require (-not [string]::IsNullOrWhiteSpace($certificate.Thumbprint)) "$Label signer thumbprint is missing"
    return $certificate
}

function Require-AbsentPath([string]$Path, [string]$Label) {
    try {
        $existing = Get-Item -Force -LiteralPath $Path
        throw "$Label must be absent before preparation: $($existing.FullName)"
    } catch [System.Management.Automation.ItemNotFoundException] {
        return
    }
}

function Download-PinnedInstaller([string]$Uri, [string]$Path, [int64]$Length) {
    $request = [Net.HttpWebRequest]::Create($Uri)
    $request.AllowAutoRedirect = $true
    $request.AutomaticDecompression = [Net.DecompressionMethods]::None
    $request.UseDefaultCredentials = $false
    $request.Credentials = $null
    $request.Timeout = 300000
    $request.ReadWriteTimeout = 300000
    $response = $null
    $responseStream = $null
    $output = $null
    $ownsOutput = $false
    $complete = $false
    try {
        $response = $request.GetResponse()
        Require ($response.ContentLength -lt 0 -or $response.ContentLength -eq $Length) 'Debugger installer declared an unexpected length'
        $responseStream = $response.GetResponseStream()
        $output = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $ownsOutput = $true
        $buffer = New-Object byte[] 65536
        [int64]$written = 0
        while (($read = $responseStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $written += $read
            Require ($written -le $Length) 'Debugger installer exceeded the pinned size'
            $output.Write($buffer, 0, $read)
        }
        Require ($written -eq $Length) 'Debugger installer did not reach the pinned size'
        $output.Flush()
        $output.Dispose()
        $output = $null
        $complete = $true
    } finally {
        if ($null -ne $output) { $output.Dispose() }
        if ($null -ne $responseStream) { $responseStream.Dispose() }
        if ($null -ne $response) { $response.Dispose() }
        $request.Abort()
        if ($ownsOutput -and -not $complete -and [IO.File]::Exists($Path)) { [IO.File]::Delete($Path) }
    }
}

Require (-not [string]::IsNullOrWhiteSpace($InputDir)) 'KURU_WINDOWS_DEBUGGER_INPUT_DIR is required'
Require (-not [string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) 'GITHUB_ENV is required'
$inputRoot = [IO.Path]::GetFullPath($InputDir)
Require-AbsentPath $inputRoot 'Debugger input directory'
[void](New-Item -ItemType Directory -Path $inputRoot)
$installer = Join-Path $inputRoot $installerName
Require-AbsentPath $installer 'Debugger installer'

Download-PinnedInstaller $installerUrl $installer $installerSize
$installerItem = Require-RegularFile $installer 'Debugger installer'
Require ($installerItem.Length -eq $installerSize) "Debugger installer size must be $installerSize bytes"
Require ((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant() -eq $installerHash) 'Debugger installer SHA-256 did not match the pinned input'
$installerCertificate = Require-MicrosoftSignature $installer 'Debugger installer'

$installerProcess = Start-Process -FilePath $installer -ArgumentList @('/features', 'OptionId.WindowsDesktopDebuggers', '/quiet', '/norestart') -Wait -PassThru
Require ($installerProcess.ExitCode -eq 0) "Windows SDK Debugging Tools setup failed: $($installerProcess.ExitCode)"

$programFilesX86 = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
Require (-not [string]::IsNullOrWhiteSpace($programFilesX86)) 'ProgramFiles(x86) is required'
$debuggerRoot = [IO.Path]::GetFullPath((Join-Path $programFilesX86 'Windows Kits\10\Debuggers\x64'))
$cdb = Join-Path $debuggerRoot 'cdb.exe'
$cdbItem = Require-RegularFile $cdb 'x64 CDB'
$resolvedCdb = (Resolve-Path -LiteralPath $cdb).Path
Require ($resolvedCdb.StartsWith("$debuggerRoot$([IO.Path]::DirectorySeparatorChar)", [StringComparison]::OrdinalIgnoreCase)) 'x64 CDB must remain below the Windows Kits Debuggers x64 root'
$cdbCertificate = Require-MicrosoftSignature $resolvedCdb 'x64 CDB'
$cdbHash = (Get-FileHash -LiteralPath $resolvedCdb -Algorithm SHA256).Hash.ToLowerInvariant()
$cdbVersion = $cdbItem.VersionInfo.FileVersion
Require (-not [string]::IsNullOrWhiteSpace($cdbVersion)) 'x64 CDB file version is missing'

Add-Content -LiteralPath $env:GITHUB_ENV "KURU_WINDOWS_CDB_PATH=$resolvedCdb"
Write-Output "Kuru Windows debugger: installer size=$($installerItem.Length) sha256=$installerHash signer-status=Valid signer-thumbprint=$($installerCertificate.Thumbprint); CDB path=$resolvedCdb version=$cdbVersion size=$($cdbItem.Length) sha256=$cdbHash signer-status=Valid signer-thumbprint=$($cdbCertificate.Thumbprint)"
