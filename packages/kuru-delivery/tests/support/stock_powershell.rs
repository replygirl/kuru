//! Module prelude for test-authored stock Windows PowerShell 5.1 scripts.
//!
//! The single source of truth for `packages/kuru-delivery/tests` and, by
//! `#[path]`, `apps/kuru-tui/tests`. A test launch with a fresh LOCALAPPDATA
//! starts stock PowerShell with a cold module-analysis cache; its first
//! Utility or Management command can then park in module discovery past the
//! test bound (#98). These are the exact imports both `install.ps1` scripts
//! perform before their first non-Core command, and the delivery contract
//! test holds them byte-identical. Use it only where a test script reaches
//! such a command before anything else imports the modules; never in a probe
//! that deliberately exercises discovery or module-path reconstruction.
#![allow(dead_code)]

/// Imports `Microsoft.PowerShell.Management` and `Microsoft.PowerShell.Utility`
/// from their exact `$PSHOME` manifests, through the always-loaded
/// `Microsoft.PowerShell.Core\Import-Module`. Each line ends in `\n`.
pub const MODULE_PRELUDE: &str = concat!(
    "$null = Microsoft.PowerShell.Core\\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\\Microsoft.PowerShell.Management\\Microsoft.PowerShell.Management.psd1')) -Verbose:$false -ErrorAction Stop\n",
    "$null = Microsoft.PowerShell.Core\\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\\Microsoft.PowerShell.Utility\\Microsoft.PowerShell.Utility.psd1')) -Verbose:$false -ErrorAction Stop\n",
);

/// `script` preceded by [`MODULE_PRELUDE`], so no statement of it runs before
/// both imports.
pub fn with_module_prelude(script: &str) -> String {
    format!("{MODULE_PRELUDE}{script}")
}
