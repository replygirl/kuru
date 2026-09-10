//! Measured official full-Dolt release assets. Refresh all sizes and hashes together.

pub const DOLT_VERSION: &str = "2.3.3";
pub(crate) const MAX_COMPRESSED: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
pub(crate) const LICENSE_BYTES: u64 = 1_212_811;
pub(crate) const LICENSE_SHA256: &str =
    "2598089ca4e9c23a7a11b330952a37f51360f2db06dfdb7d699de1bca1371386";

#[derive(Clone, Copy, Debug)]
pub(crate) struct Asset<'a> {
    pub target: &'a str,
    pub stem: &'a str,
    pub url: &'a str,
    pub compressed_bytes: u64,
    pub archive_sha256: &'a str,
    pub expanded_bytes: u64,
    pub executable_bytes: u64,
    pub executable_sha256: &'a str,
    pub license_bytes: u64,
    pub license_sha256: &'a str,
}

// https://github.com/dolthub/dolt/releases/tag/v2.3.3
// Upstream commit: 79caf258c32e638868f17a803eb2c2779c0507ab.
// Archive digests come from official release metadata; payloads were independently
// measured after matching those digests. They are not a signature/attestation.
pub(crate) const ASSETS: [Asset<'static>; 4] = [
    Asset {
        target: "aarch64-apple-darwin",
        stem: "dolt-darwin-arm64",
        url: "https://github.com/dolthub/dolt/releases/download/v2.3.3/dolt-darwin-arm64.tar.gz",
        compressed_bytes: 41_132_675,
        archive_sha256: "55c11d34df78d7583f1130a1adef7763340f2aade33e68ccb0046854134ad08b",
        expanded_bytes: 120_064_000,
        executable_bytes: 118_846_882,
        executable_sha256: "84e8724957dd7b24202d5cbe0e5f6eba37f1f17e43f9094e92ba328b47413a6d",
        license_bytes: LICENSE_BYTES,
        license_sha256: LICENSE_SHA256,
    },
    Asset {
        target: "x86_64-apple-darwin",
        stem: "dolt-darwin-amd64",
        url: "https://github.com/dolthub/dolt/releases/download/v2.3.3/dolt-darwin-amd64.tar.gz",
        compressed_bytes: 43_603_373,
        archive_sha256: "e33f4fa00032054e38da78b31314f8e93fa9eb950c587ac5f29ba5c6402b3f2a",
        expanded_bytes: 127_662_080,
        executable_bytes: 126_442_936,
        executable_sha256: "cb37ed2489c15d62cd0a054c3c594337ca1b758a3866b63eef845490e58f0776",
        license_bytes: LICENSE_BYTES,
        license_sha256: LICENSE_SHA256,
    },
    Asset {
        target: "aarch64-unknown-linux-gnu",
        stem: "dolt-linux-arm64",
        url: "https://github.com/dolthub/dolt/releases/download/v2.3.3/dolt-linux-arm64.tar.gz",
        compressed_bytes: 40_750_254,
        archive_sha256: "850a880aece6587cb9251ea0f07eb51fcc0a37450471fd89e03ac2fba1fdaed3",
        expanded_bytes: 120_033_280,
        executable_bytes: 118_817_184,
        executable_sha256: "56c20166b6da38fd5b5189358443fd5e450ec634fdad3cc2ca239ecf959fb5f1",
        license_bytes: LICENSE_BYTES,
        license_sha256: LICENSE_SHA256,
    },
    Asset {
        target: "x86_64-unknown-linux-gnu",
        stem: "dolt-linux-amd64",
        url: "https://github.com/dolthub/dolt/releases/download/v2.3.3/dolt-linux-amd64.tar.gz",
        compressed_bytes: 43_971_030,
        archive_sha256: "4acd730a4c53991996854a72fbb1add102b0a583bd07411320efb65037a43d9d",
        expanded_bytes: 127_897_600,
        executable_bytes: 126_673_640,
        executable_sha256: "ec7c200f0c92516349c6e607635f6f8717e22abe31275ac326125d32356bc786",
        license_bytes: LICENSE_BYTES,
        license_sha256: LICENSE_SHA256,
    },
];

pub(crate) fn host_asset() -> anyhow::Result<Asset<'static>> {
    select_asset(std::env::consts::OS, std::env::consts::ARCH)
}

fn select_asset(os: &str, arch: &str) -> anyhow::Result<Asset<'static>> {
    let index = match (os, arch) {
        ("macos", "aarch64") => 0,
        ("macos", "x86_64") => 1,
        ("linux", "aarch64") => 2,
        ("linux", "x86_64") => 3,
        _ => anyhow::bail!(
            "Dolt memory does not support {os}/{arch}; supported platforms are macOS and Linux on arm64 and x86_64"
        ),
    };
    Ok(ASSETS[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_exact_release_targets_with_bounded_payloads() {
        for ((os, arch), asset) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "x86_64"),
        ]
        .into_iter()
        .zip(ASSETS)
        {
            assert_eq!(select_asset(os, arch).unwrap().target, asset.target);
            assert!(asset.compressed_bytes < MAX_COMPRESSED);
            assert!(asset.expanded_bytes < MAX_EXPANDED);
            assert!(asset.executable_bytes + LICENSE_BYTES < asset.expanded_bytes);
            assert!(asset.url.ends_with(&format!("/{}.tar.gz", asset.stem)));
            for digest in [
                asset.archive_sha256,
                asset.executable_sha256,
                asset.license_sha256,
            ] {
                assert_eq!(digest.len(), 64);
                assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
            }
        }
        assert!(select_asset("windows", "x86_64").is_err());
        assert!(select_asset("linux", "riscv64").is_err());
    }
}
