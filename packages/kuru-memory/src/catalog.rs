//! Generated from the single measured release manifest at build time.
pub(crate) const MAX_COMPRESSED: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
pub(crate) const EMBEDDED_ARCHIVE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/dolt.archive"));

#[derive(Clone, Copy, Debug)]
pub(crate) struct Asset<'a> {
    pub target: &'a str,
    pub stem: &'a str,
    pub format: &'a str,
    pub executable_name: &'a str,
    pub compressed_bytes: u64,
    pub archive_sha256: &'a str,
    pub expanded_bytes: u64,
    pub executable_bytes: u64,
    pub executable_sha256: &'a str,
    pub license_bytes: u64,
    pub license_sha256: &'a str,
    /// Third-party notices beside `LICENSES`; empty for upstream archives.
    pub notices: &'a [Notice<'a>],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Notice<'a> {
    pub name: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
}

include!(concat!(env!("OUT_DIR"), "/dolt_catalog.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn embedded_bytes_match_the_selected_target_and_versioned_catalog() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../support/dolt-assets.json")).unwrap();
        assert_eq!(manifest["version"], DOLT_VERSION);
        // Unpinned built assets are refused as inputs and absent from the catalog.
        let pinned = manifest["assets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|asset| asset["archive_sha256"] != "unpinned")
            .count();
        assert_eq!(ASSETS.len(), pinned);
        assert!(ASSETS.len() >= 5);
        assert!(
            ASSETS
                .iter()
                .any(|asset| asset.target == BUNDLED_ASSET.target)
        );
        assert_eq!(
            EMBEDDED_ARCHIVE.len() as u64,
            BUNDLED_ASSET.compressed_bytes
        );
        let digest: String = Sha256::digest(EMBEDDED_ARCHIVE)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(digest, BUNDLED_ASSET.archive_sha256);
        assert!(BUNDLED_ASSET.target.starts_with(std::env::consts::ARCH));
    }
}
