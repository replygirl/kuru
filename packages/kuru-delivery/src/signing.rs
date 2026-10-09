//! Private copies and native publisher checks for release executables.

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{
    Directory, NameRetention, Privacy, Publication, PublicationPhase, regular_file_info,
};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    })
}

fn bounded(input: &mut File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input
        .take(crate::archive::MAX_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= crate::archive::MAX_ARCHIVE_BYTES,
        "signing executable must be nonempty and within the release size limit"
    );
    Ok(bytes)
}

/// Copy a verified build input into an owner-only signing directory. Existing
/// executable names are never replaced, including on retries after signing.
/// Cargo's source may have hard links; only the separate copy is writable.
pub fn prepare(binary: &Path, target: &str, output: &Path) -> Result<PathBuf> {
    let target = crate::targets::find(target)?;
    let binary = absolute(binary)?;
    let parent = Directory::open(
        binary.parent().context("binary has no parent")?,
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    let name = binary.file_name().context("binary has no filename")?;
    let mut input = crate::archive::open_build_input(&parent, name)?;
    let before = regular_file_info(&input)?;
    ensure!(
        before.len > 0 && before.len <= crate::archive::MAX_ARCHIVE_BYTES as u64,
        "signing executable must be nonempty and within the release size limit"
    );
    let bytes = bounded(&mut input)?;
    crate::archive::verify_build_snapshot(&parent, name, &mut input, before, &bytes)?;
    let output = Directory::ensure_private(&absolute(output)?)?;
    let staging = crate::staging::Stage::create(&output, ".kuru-signing-")?;
    let stage = staging.directory();
    let name = OsStr::new(target.executable);
    let mut copy = stage.create_new(name)?;
    copy.write_all(&bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        copy.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    kuru_platform::fs::make_executable(&copy)?;
    copy.sync_all()?;
    // A possibly completed publication is reconciled against this exact held
    // copy; an existing signing output never authorizes replacement.
    if let Err(error) = output.publish_file(stage, name, &copy, name, Publication::New)
        && (error.phase != PublicationPhase::Uncertain || output.verify(name, &copy).is_err())
    {
        return Err(error.into());
    }
    drop(copy);
    staging
        .finish()
        .context("signing copy exists, but private staging cleanup failed")?;
    Ok(output.path().join(name))
}

/// Reopen the result after the signer (which may replace its inode), check the
/// target and native publisher policy, and prove verification observed unchanged
/// bytes. `publisher` is an Apple Team ID or exact Authenticode Subject.
pub async fn verify(binary: &Path, target: &str, publisher: &str) -> Result<()> {
    let target = crate::targets::find(target)?;
    let binary = absolute(binary)?;
    let parent = Directory::open(
        binary.parent().context("binary has no parent")?,
        Privacy::OwnerOnly,
        NameRetention::Movable,
    )?;
    let name = binary.file_name().context("binary has no filename")?;
    ensure!(
        name == target.executable,
        "unexpected signing executable name"
    );
    let mut input = parent.read(name)?;
    let before = regular_file_info(&input)?;
    let bytes = bounded(&mut input)?;
    native_policy(&binary, target.os, publisher).await?;
    crate::archive::verify_build_snapshot(&parent, name, &mut input, before, &bytes)?;
    parent.verify(name, &input)?;
    Ok(())
}

async fn native_policy(binary: &Path, os: &str, publisher: &str) -> Result<()> {
    // Each signed platform is verified only on its native host. Windows x64
    // verifies ARM64 PE signatures without executing those images.
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = binary;
    match os {
        "linux" => Ok(()),
        "macos" => {
            ensure!(
                publisher.len() == 10
                    && publisher
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
                "Apple publisher must be a ten-character Team ID"
            );
            #[cfg(target_os = "macos")]
            {
                let requirement = format!(
                    "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"{publisher}\""
                );
                let mut command = crate::command::Command::new("/usr/bin/codesign");
                command
                    .args(["--verify", "--strict", "--verbose=2", "-R", &requirement])
                    .arg(binary);
                checked_command(&mut command)
                    .await
                    .context("Developer ID signature verification failed")?;
                let mut command = crate::command::Command::new("/usr/bin/codesign");
                command.args(["--display", "--verbose=4"]).arg(binary);
                let metadata = checked_command(&mut command).await?;
                apple_metadata(std::str::from_utf8(&metadata.stderr)?, publisher)
            }
            #[cfg(not(target_os = "macos"))]
            bail!("macOS signature verification requires a native macOS host")
        }
        "windows" => {
            ensure!(
                !publisher.is_empty()
                    && publisher.len() <= 4096
                    && !publisher.contains(['\r', '\n', '\0']),
                "Windows publisher must be an exact nonempty certificate Subject"
            );
            #[cfg(windows)]
            {
                let program = kuru_platform::windows::process::system_directory()?
                    .join("WindowsPowerShell/v1.0/powershell.exe");
                let mut command = crate::command::Command::new(program);
                command
                    .args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        WINDOWS_VERIFY,
                    ])
                    .env_remove("PSModulePath")
                    .env("KURU_SIGNING_BINARY", binary);
                let metadata = checked_command(&mut command)
                    .await
                    .context("Authenticode signature verification failed")?;
                windows_metadata(&serde_json::from_slice(&metadata.stdout)?, publisher)
            }
            #[cfg(not(windows))]
            bail!("Windows signature verification requires a native Windows host")
        }
        _ => bail!("unsupported signature verification platform"),
    }
}

#[cfg(any(target_os = "macos", windows))]
async fn checked_command(command: &mut crate::command::Command) -> Result<std::process::Output> {
    let output =
        crate::command::bounded_output(command, std::time::Duration::from_secs(120), 64 * 1024)
            .await?;
    ensure!(
        output.status.success(),
        "native signature verifier failed ({})",
        output.status
    );
    Ok(output)
}

#[cfg(any(target_os = "macos", test))]
fn apple_metadata(metadata: &str, publisher: &str) -> Result<()> {
    let field = |prefix: &str| {
        metadata
            .lines()
            .filter_map(move |line| line.strip_prefix(prefix))
            .collect::<Vec<_>>()
    };
    ensure!(
        field("TeamIdentifier=") == [publisher],
        "Developer ID Team ID differs from configured publisher"
    );
    ensure!(
        metadata
            .lines()
            .any(|line| line.starts_with("Authority=Developer ID Application: ")),
        "signature is missing Developer ID Application authority"
    );
    let flags = field("CodeDirectory ");
    ensure!(
        flags.len() == 1
            && flags[0].split_whitespace().any(|field| {
                field
                    .strip_prefix("flags=")
                    .and_then(|flags| flags.split_once('('))
                    .is_some_and(|(hex, names)| {
                        u32::from_str_radix(hex.trim_start_matches("0x"), 16)
                            .is_ok_and(|flags| flags & 0x10000 != 0)
                            && names
                                .trim_end_matches(')')
                                .split(',')
                                .any(|name| name == "runtime")
                    })
            }),
        "signature requires hardened runtime"
    );
    let timestamp = field("Timestamp=");
    ensure!(
        timestamp.len() == 1
            && !timestamp[0].is_empty()
            && !timestamp[0].eq_ignore_ascii_case("none"),
        "signature requires a secure timestamp"
    );
    Ok(())
}

#[cfg(any(windows, test))]
fn windows_metadata(metadata: &serde_json::Value, publisher: &str) -> Result<()> {
    ensure!(
        metadata["status"].as_str() == Some("Valid"),
        "Authenticode signature is not valid"
    );
    ensure!(
        metadata["signature_type"].as_str() == Some("Authenticode"),
        "release executable requires an embedded Authenticode signature"
    );
    ensure!(
        metadata["subject"].as_str() == Some(publisher),
        "Authenticode publisher differs from configured Subject"
    );
    ensure!(
        metadata["timestamp"].as_bool() == Some(true),
        "Authenticode signature requires a timestamp certificate"
    );
    ensure!(
        metadata["timestamp_eku"].as_bool() == Some(true),
        "Authenticode timestamp certificate requires time-stamping usage"
    );
    Ok(())
}

#[cfg(windows)]
const WINDOWS_VERIFY: &str = r#"
$ErrorActionPreference = 'Stop'
$null = Microsoft.PowerShell.Core\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1')) -ErrorAction Stop
$null = Microsoft.PowerShell.Core\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1')) -ErrorAction Stop
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$signature = Microsoft.PowerShell.Security\Get-AuthenticodeSignature -LiteralPath $env:KURU_SIGNING_BINARY
$timestamp = $signature.TimeStamperCertificate
$usage = $false
if ($null -ne $timestamp) {
    foreach ($extension in $timestamp.Extensions) {
        if ($extension.Oid.Value -eq '2.5.29.37') {
            foreach ($oid in $extension.EnhancedKeyUsages) {
                if ($oid.Value -eq '1.3.6.1.5.5.7.3.8') { $usage = $true }
            }
        }
    }
}
$subject = if ($null -ne $signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { '' }
@{ status = $signature.Status.ToString(); signature_type = $signature.SignatureType.ToString(); subject = $subject; timestamp = ($null -ne $timestamp); timestamp_eku = $usage } | Microsoft.PowerShell.Utility\ConvertTo-Json -Compress
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_policy_requires_team_developer_id_runtime_and_secure_timestamp() {
        let valid = "CodeDirectory v=20500 size=100 flags=0x10000(runtime) hashes=2+0 location=embedded\nAuthority=Developer ID Application: Fixture (ABCDEFGHIJ)\nTeamIdentifier=ABCDEFGHIJ\nTimestamp=Oct 9, 2026 at 12:00:00\n";
        apple_metadata(valid, "ABCDEFGHIJ").unwrap();
        for bad in [
            valid.replace("TeamIdentifier=ABCDEFGHIJ", "TeamIdentifier=KLMNOPQRST"),
            valid.replace("Developer ID Application:", "Apple Development:"),
            valid.replace("flags=0x10000(runtime)", "flags=0x0(none)"),
            valid.replace("flags=0x10000(runtime)", "flags=0x0(runtime)"),
            valid.replace("Timestamp=Oct 9, 2026 at 12:00:00", "Timestamp=none"),
            valid.replace("Timestamp=Oct 9, 2026 at 12:00:00\n", ""),
            format!("{valid}TeamIdentifier=ABCDEFGHIJ\n"),
            format!("{valid}Timestamp=other\n"),
        ] {
            assert!(
                apple_metadata(&bad, "ABCDEFGHIJ").is_err(),
                "accepted {bad}"
            );
        }
    }

    #[test]
    fn windows_policy_requires_valid_exact_publisher_and_timestamp_usage() {
        let valid = serde_json::json!({"status":"Valid", "signature_type":"Authenticode", "subject":"CN=Fixture Publisher", "timestamp":true, "timestamp_eku":true});
        windows_metadata(&valid, "CN=Fixture Publisher").unwrap();
        for (key, value) in [
            ("status", serde_json::json!("NotSigned")),
            ("signature_type", serde_json::json!("Catalog")),
            ("subject", serde_json::json!("CN=Another Publisher")),
            ("timestamp", serde_json::json!(false)),
            ("timestamp_eku", serde_json::json!(false)),
        ] {
            let mut bad = valid.clone();
            bad[key] = value;
            assert!(windows_metadata(&bad, "CN=Fixture Publisher").is_err());
        }
        assert!(windows_metadata(&serde_json::json!({}), "CN=Fixture Publisher").is_err());
    }
}
