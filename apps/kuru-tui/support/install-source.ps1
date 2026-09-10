# Source installation is an app-owned mise task. Release installation uses the
# independent delivery bootstrap and never requires this compiler toolchain.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
# A caller may be a Git hook with repository-selection variables set. Build
# tools must observe this checkout rather than the caller's Git directory.
foreach ($selector in @(
    'GIT_ALTERNATE_OBJECT_DIRECTORIES', 'GIT_CONFIG', 'GIT_CONFIG_PARAMETERS',
    'GIT_CONFIG_COUNT', 'GIT_OBJECT_DIRECTORY', 'GIT_DIR', 'GIT_WORK_TREE',
    'GIT_IMPLICIT_WORK_TREE', 'GIT_GRAFT_FILE', 'GIT_INDEX_FILE',
    'GIT_NO_REPLACE_OBJECTS', 'GIT_REPLACE_REF_BASE', 'GIT_PREFIX',
    'GIT_SHALLOW_FILE', 'GIT_COMMON_DIR'
)) {
    Remove-Item -LiteralPath ("Env:" + $selector) -ErrorAction SilentlyContinue
}
$env:MISE_NO_HOOKS = '1'
& mise -C $repo install rust
if ($LASTEXITCODE -ne 0) { throw 'Could not install the pinned Rust toolchain.' }
$hostTarget = (& mise -C $repo exec rust -- rustc --print host-tuple).Trim()
if ($LASTEXITCODE -ne 0 -or $hostTarget -ne 'x86_64-pc-windows-msvc') {
    throw 'Source installation requires the supported native x86_64 MSVC toolchain.'
}
if ($env:CARGO_BUILD_TARGET -and $env:CARGO_BUILD_TARGET -ne 'host' -and $env:CARGO_BUILD_TARGET -ne $hostTarget) {
    throw "Source installation runs on $hostTarget; use the build task to cross-compile."
}
# All target-aware consumer builds below receive the native triple explicitly;
# host delivery tools must never see the convenience sentinel as a Rust triple.
Remove-Item Env:CARGO_BUILD_TARGET -ErrorAction SilentlyContinue
$env:MISE_TASK_RUN_AUTO_INSTALL = 'false'
& mise -C $repo run //apps/kuru-tui:build:release -- --target $hostTarget
if ($LASTEXITCODE -ne 0) { throw 'Kuru source build failed.' }
$targetDirectory = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
if (-not [IO.Path]::IsPathRooted($targetDirectory)) { $targetDirectory = Join-Path $repo $targetDirectory }
$binary = Join-Path $targetDirectory "$hostTarget/release/kuru.exe"
$destination = if ($env:KURU_INSTALL_DIR) { $env:KURU_INSTALL_DIR } else {
    if (-not $env:LOCALAPPDATA) { throw 'Set KURU_INSTALL_DIR or LOCALAPPDATA for source installation.' }
    Join-Path $env:LOCALAPPDATA 'Programs/kuru/bin'
}
# Resolve a relative install directory against the caller, not the checkout.
$destination = [IO.Path]::GetFullPath($destination)
& mise -C $repo run //packages/kuru-delivery:tool -- install-local --binary $binary --install-dir $destination --target $hostTarget
if ($LASTEXITCODE -ne 0) { throw 'Source installation failed; inspect its reported publication state before retrying.' }
& (Join-Path $destination 'kuru.exe') --version
if ($LASTEXITCODE -ne 0) { throw 'Installed Kuru did not report its version.' }
Write-Host "Installed at $destination\kuru.exe; add $destination to PATH."
