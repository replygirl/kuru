use anyhow::{Context, Result, ensure};
use kuru_delivery::command::Command;
use std::{collections::BTreeSet, fs, path::PathBuf, time::Duration};

pub struct UpdateProfiles {
    directory: PathBuf,
    prefix: String,
    _reservation: tempfile::NamedTempFile,
}

impl UpdateProfiles {
    pub fn observe(command: &mut Command) -> Result<Option<Self>> {
        // Only Windows hands publication to another executable. Shipping
        // acceptance explicitly selects a noninstrumented executable.
        if !cfg!(windows) || std::env::var_os("KURU_EMBEDDED_TEST_BINARY").is_some() {
            return Ok(None);
        }
        let Some(destination) = std::env::var_os("LLVM_PROFILE_FILE") else {
            return Ok(None);
        };
        let destination = PathBuf::from(destination);
        ensure!(
            destination.is_absolute(),
            "coverage destination must be absolute"
        );
        let directory = destination
            .parent()
            .context("coverage directory")?
            .to_owned();
        let reservation = tempfile::Builder::new()
            .prefix("kuru-update-")
            .tempfile_in(&directory)?;
        let prefix = format!(
            "{}-",
            reservation
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .context("profile reservation filename")?
        );
        // Keep every profile in the runner's existing *.profraw merge inventory.
        command.env(
            "LLVM_PROFILE_FILE",
            directory.join(format!("{prefix}%p-%m.profraw")),
        );
        Ok(Some(Self {
            directory,
            prefix,
            _reservation: reservation,
        }))
    }

    pub async fn verify(self) -> Result<()> {
        // The normal update branch launches only its trusted helper. Separate
        // PIDs prove both processes retained the destination across the handoff.
        // The helper exits after its parent, within the existing cleanup bound.
        let mut observed = 0;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let mut processes = BTreeSet::new();
                for entry in fs::read_dir(&self.directory)? {
                    let entry = entry?;
                    let filename = entry.file_name();
                    let Some(filename) = filename.to_str() else {
                        continue;
                    };
                    let Some(suffix) = filename.strip_prefix(&self.prefix) else {
                        continue;
                    };
                    ensure!(
                        suffix.ends_with(".profraw"),
                        "unexpected update profile filename"
                    );
                    let (pid, _) = suffix.split_once('-').context("profile process ID")?;
                    let pid: u32 = pid.parse().context("invalid profile process ID")?;
                    let metadata = entry.metadata()?;
                    ensure!(metadata.is_file(), "update profile must be a regular file");
                    if metadata.len() > 0 {
                        processes.insert(pid);
                    }
                }
                ensure!(
                    processes.len() <= 2,
                    "unexpected extra update profile process"
                );
                observed = processes.len();
                if processes.len() == 2 {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .with_context(|| format!(
            "normal app and update helper did not both emit profiles in the runner directory: observed {observed} of 2 processes"
        ))?
    }
}
