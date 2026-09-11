# Native acceptance of Cargo's actual build-script boundary after source install.
# Reuse the shipping target/profile; do not run Cargo recursively inside tests.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$target = 'x86_64-pc-windows-msvc'
if ((& rustc --print host-tuple).Trim() -ne $target -or $LASTEXITCODE -ne 0) {
    throw 'Bundle build acceptance requires native Windows x64/MSVC.'
}
$catalog = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'dolt-assets.json') | ConvertFrom-Json
$assets = @($catalog.assets | Where-Object { $_.target -ceq $target })
if ($assets.Count -ne 1 -or $assets[0].archive_sha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw 'Expected exactly one pinned native archive.'
}
$asset = $assets[0]
$mirror = if ($env:KURU_DOLT_BUNDLE_DIR) { $env:KURU_DOLT_BUNDLE_DIR } else { Join-Path $repo 'target/kuru-bundles' }
if (-not [IO.Path]::IsPathRooted($mirror)) { throw 'The prepared mirror must be absolute.' }
$archiveName = "$($asset.archive_sha256).archive"
$archive = Join-Path $mirror $archiveName
$originalHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
if ($originalHash -cne $asset.archive_sha256 -or (Get-Item -LiteralPath $archive).Length -ne $asset.compressed_bytes) {
    throw 'Prepare the exact native bundle before running build acceptance.'
}
if (-not $env:KURU_EMBEDDED_TEST_BINARY) { throw 'Select the already source-installed executable with KURU_EMBEDDED_TEST_BINARY.' }
$installed = $env:KURU_EMBEDDED_TEST_BINARY
$installedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installed).Hash
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('kuru-build-input-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($scratch) | Out-Null
$savedMirror = $env:KURU_DOLT_BUNDLE_DIR

function Invoke-Build([string] $label, [string] $inputMirror, [string] $expectedFailure) {
    $env:KURU_DOLT_BUNDLE_DIR = $inputMirror
    # Windows PowerShell wraps native stderr as ErrorRecords. Inspect Cargo's
    # actual status and text, rather than treating its ordinary progress as failure.
    try {
        $ErrorActionPreference = 'Continue'
        $output = & cargo build -p kuru --release --all-features --locked --offline --target $target 2>&1
        $status = $LASTEXITCODE
    } finally { $ErrorActionPreference = 'Stop' }
    $message = $output | Out-String -Width 4096
    Write-Host $message
    if ($expectedFailure) {
        if ($status -eq 0 -or -not $message.Contains('failed to run custom build command for `kuru-memory') -or
            -not $message.Contains($expectedFailure) -or -not $message.Contains((Join-Path $inputMirror $archiveName))) {
            throw "$label did not fail at the intended memory build-script boundary (status $status)."
        }
    } elseif ($status -ne 0) { throw "Valid prepared offline build failed (status $status)." }
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $installed).Hash -cne $installedHash -or
        (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant() -cne $originalHash) {
        throw "$label changed the installed executable or original prepared archive."
    }
    Write-Host "Verified offline Cargo boundary: $label"
}

Push-Location -LiteralPath $repo
try {
    $missing = Join-Path $scratch 'missing'
    $corrupt = Join-Path $scratch 'corrupt'
    [IO.Directory]::CreateDirectory($missing) | Out-Null
    [IO.Directory]::CreateDirectory($corrupt) | Out-Null
    Invoke-Build 'missing archive' $missing 'open checked build-input file'
    $bad = Join-Path $corrupt $archiveName
    [IO.File]::Copy($archive, $bad, $false)
    $file = [IO.File]::Open($bad, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $first = $file.ReadByte()
        $file.Position = 0
        $file.WriteByte([byte]($first -bxor 1))
        $file.Flush($true)
    } finally { $file.Dispose() }
    if ((Get-Item -LiteralPath $bad).Length -ne $asset.compressed_bytes) { throw 'Corrupt fixture changed archive size.' }
    Invoke-Build 'same-size corrupt archive' $corrupt 'prepared Dolt archive checksum mismatch'
    Invoke-Build 'valid prepared archive' $mirror ''
} finally {
    $env:KURU_DOLT_BUNDLE_DIR = $savedMirror
    Pop-Location
    Remove-Item -LiteralPath $scratch -Recurse -Force
}
