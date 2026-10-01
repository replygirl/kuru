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
# Load the stock modules this entrypoint and its forwarded bootstrap use from
# their exact PSHOME manifests before the first command outside
# Microsoft.PowerShell.Core. A bare first cmdlet would enter module
# auto-discovery, which can stall indefinitely on a fresh profile's cold
# analysis cache. Other modules keep ordinary autoloading.
$null = Microsoft.PowerShell.Core\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\Microsoft.PowerShell.Management\Microsoft.PowerShell.Management.psd1')) -Verbose:$false -ErrorAction Stop
$null = Microsoft.PowerShell.Core\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1')) -Verbose:$false -ErrorAction Stop
$kuruRepo = Split-Path -Parent $PSScriptRoot
if ($Source) {
    if ($Version -or $Target -or $ReleaseBase -or $Recover) {
        throw 'Source installation does not accept release-selection or recovery options.'
    }
    $previousInstallDir = $env:KURU_INSTALL_DIR
    $previousNoHooks = $env:MISE_NO_HOOKS
    $previousAutoInstall = $env:MISE_TASK_RUN_AUTO_INSTALL
    $previousMbx = $env:KURU_MBX
    try {
        # Mise activates task tools before the app-owned script can scope them.
        $env:MISE_NO_HOOKS = '1'
        $env:MISE_TASK_RUN_AUTO_INSTALL = 'false'
        # The mr-boxington build cache is a maintainer tool.
        $env:KURU_MBX = '0'
        $selectedInstallDir = if ($InstallDir) { $InstallDir } else { $previousInstallDir }
        if ($selectedInstallDir) {
            # Normalize before mise enters the package-owned task directory.
            $env:KURU_INSTALL_DIR = [IO.Path]::GetFullPath($selectedInstallDir)
        }
        & mise -C $kuruRepo run '//apps/kuru-tui:install'
        if ($LASTEXITCODE -ne 0) { throw "Source installation failed ($LASTEXITCODE)." }
    } finally {
        $env:KURU_INSTALL_DIR = $previousInstallDir
        $env:MISE_NO_HOOKS = $previousNoHooks
        $env:MISE_TASK_RUN_AUTO_INSTALL = $previousAutoInstall
        $env:KURU_MBX = $previousMbx
    }
} else {
    $options = @{}
    foreach ($name in @('Version', 'Target', 'ReleaseBase', 'InstallDir', 'Recover')) {
        if ($PSBoundParameters.ContainsKey($name)) { $options[$name] = $PSBoundParameters[$name] }
    }
    & (Join-Path $kuruRepo 'packages/kuru-delivery/support/install.ps1') @options
}
