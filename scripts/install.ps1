[CmdletBinding()]
param(
    [switch]$Source,
    [string]$Version,
    [string]$Target,
    [string]$ReleaseBase,
    [string]$InstallDir,
    [switch]$Recover
)
$ErrorActionPreference = 'Stop'
$kuruRepo = Split-Path -Parent $PSScriptRoot
if ($Source) {
    if ($Version -or $Target -or $ReleaseBase -or $Recover) {
        throw 'Source installation does not accept release-selection or recovery options.'
    }
    $previousInstallDir = $env:KURU_INSTALL_DIR
    try {
        $selectedInstallDir = if ($InstallDir) { $InstallDir } else { $previousInstallDir }
        if ($selectedInstallDir) {
            # Normalize before mise enters the package-owned task directory.
            $env:KURU_INSTALL_DIR = [IO.Path]::GetFullPath($selectedInstallDir)
        }
        & mise -C $kuruRepo run '//apps/kuru-tui:install'
        if ($LASTEXITCODE -ne 0) { throw "Source installation failed ($LASTEXITCODE)." }
    } finally {
        $env:KURU_INSTALL_DIR = $previousInstallDir
    }
} else {
    $options = @{}
    foreach ($name in @('Version', 'Target', 'ReleaseBase', 'InstallDir', 'Recover')) {
        if ($PSBoundParameters.ContainsKey($name)) { $options[$name] = $PSBoundParameters[$name] }
    }
    & (Join-Path $kuruRepo 'packages/kuru-delivery/support/install.ps1') @options
}
