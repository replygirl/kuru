use std::path::{Path, PathBuf};

/// CLI fixtures use ordinary user configuration and the package's verified
/// offline cache; no production-only authority or ambient user store is used.
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
