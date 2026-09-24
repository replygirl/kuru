use std::path::{Path, PathBuf};

use anyhow::Context as _;

/// Retires managed memory owners created by one application fixture before its
/// temporary data root is removed. The service intentionally outlives its last
/// client for a warm idle interval, so subprocess fixtures must own this final
/// maintenance step rather than leave independent Dolt processes accumulating
/// across the parallel application suite.
pub struct ServiceCleanup {
    data: Vec<PathBuf>,
    root: Option<tempfile::TempDir>,
    pending: bool,
}

impl ServiceCleanup {
    pub fn new(root: tempfile::TempDir, data: &Path) -> Self {
        Self {
            data: vec![data.to_owned()],
            root: Some(root),
            pending: true,
        }
    }

    #[allow(
        dead_code,
        reason = "each integration-test crate compiles this shared support module independently"
    )]
    pub fn path(&self) -> &Path {
        self.root.as_ref().expect("fixture root retained").path()
    }

    #[allow(
        dead_code,
        reason = "only fixtures with more than one managed data root use this extension"
    )]
    pub fn add_data(&mut self, data: &Path) {
        assert!(self.pending, "cannot extend completed fixture cleanup");
        self.data.push(data.to_owned());
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if !std::mem::replace(&mut self.pending, false) {
            return Ok(());
        }
        if let Err(error) = self.retire() {
            let retained = self
                .root
                .take()
                .expect("fixture root retained before cleanup")
                .keep();
            return Err(error.context(format!(
                "managed-memory cleanup failed; fixture root retained in place at {retained:?}"
            )));
        }
        Ok(())
    }

    fn retire(&self) -> anyhow::Result<()> {
        let mut projects = Vec::new();
        for data in &self.data {
            let memory = data.join("memory");
            let entries = match std::fs::read_dir(&memory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("inspect fixture memory root {memory:?}"));
                }
            };
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if entry.file_type()?.is_dir()
                    && name.len() == 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    projects.push((data.clone(), format!("project/{name}")));
                }
            }
        }
        if projects.is_empty() {
            return Ok(());
        }
        projects.sort();
        std::thread::Builder::new()
            .name("kuru-fixture-memory-cleanup".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("create managed-memory cleanup runtime")?;
                runtime.block_on(async move {
                    for (data, scope) in projects {
                        let options = kuru_memory::OpenOptions::new(data, scope);
                        kuru_memory::test_support::retire_idle_service(&options)
                            .await
                            .with_context(|| {
                                format!("retire fixture memory owner for {}", options.project_scope)
                            })?;
                    }
                    Ok::<(), anyhow::Error>(())
                })
            })?
            .join()
            .map_err(|_| anyhow::anyhow!("managed-memory fixture cleanup thread panicked"))?
    }
}

impl Drop for ServiceCleanup {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            if std::thread::panicking() {
                eprintln!("managed-memory fixture cleanup failed: {error:#}");
            } else {
                panic!("managed-memory fixture cleanup failed: {error:#}");
            }
        }
    }
}

/// CLI fixtures use ordinary user configuration and the package's verified
/// offline cache; no production-only authority or ambient user store is used.
#[allow(
    dead_code,
    reason = "each integration-test crate compiles this shared support module independently"
)]
pub fn configuration(root: &Path) -> anyhow::Result<PathBuf> {
    let directory = root.join("config");
    std::fs::create_dir_all(directory.join("kuru"))?;
    let memory = kuru_core::MemoryConfig {
        cache_dir: Some(kuru_memory::test_support::cache_dir()),
        offline: true,
        ..Default::default()
    };
    let config = std::collections::BTreeMap::from([("memory", memory)]);
    std::fs::write(
        directory.join("kuru/config.toml"),
        toml::to_string(&config)?,
    )?;
    Ok(directory)
}
