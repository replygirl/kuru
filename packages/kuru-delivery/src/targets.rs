//! Authoritative native release names and formats shared by every delivery path.

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveFormat {
    TarGz,
    Zip,
}

impl ArchiveFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::TarGz => "tar.gz",
            Self::Zip => "zip",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target {
    pub triple: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub executable: &'static str,
    pub format: ArchiveFormat,
}

pub const CATALOG: [Target; 4] = [
    Target {
        triple: "aarch64-apple-darwin",
        os: "macos",
        arch: "aarch64",
        executable: "kuru",
        format: ArchiveFormat::TarGz,
    },
    Target {
        triple: "aarch64-unknown-linux-gnu",
        os: "linux",
        arch: "aarch64",
        executable: "kuru",
        format: ArchiveFormat::TarGz,
    },
    Target {
        triple: "x86_64-unknown-linux-gnu",
        os: "linux",
        arch: "x86_64",
        executable: "kuru",
        format: ArchiveFormat::TarGz,
    },
    Target {
        triple: "x86_64-pc-windows-msvc",
        os: "windows",
        arch: "x86_64",
        executable: "kuru.exe",
        format: ArchiveFormat::Zip,
    },
];

/// Compatibility projection; the catalog above owns all target metadata.
pub const TARGETS: [&str; CATALOG.len()] = {
    let mut names = [""; CATALOG.len()];
    let mut index = 0;
    while index < CATALOG.len() {
        names[index] = CATALOG[index].triple;
        index += 1;
    }
    names
};

pub fn find(triple: &str) -> Result<&'static Target> {
    CATALOG
        .iter()
        .find(|target| target.triple == triple)
        .ok_or_else(|| anyhow::anyhow!("unsupported release target: {triple}"))
}

pub fn for_platform(os: &str, arch: &str) -> Result<&'static Target> {
    if let Some(target) = CATALOG
        .iter()
        .find(|target| target.os == os && target.arch == arch)
    {
        return Ok(target);
    }
    bail!("unsupported release platform: {os}/{arch}")
}

pub fn host() -> Result<&'static Target> {
    for_platform(std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_platform_has_one_consistent_release_name() {
        let mut names = std::collections::HashSet::new();
        for target in &CATALOG {
            assert!(names.insert(target.triple));
            assert_eq!(find(target.triple).unwrap(), target);
            assert_eq!(for_platform(target.os, target.arch).unwrap(), target);
        }
        let windows = for_platform("windows", "x86_64").unwrap();
        assert_eq!(windows.executable, "kuru.exe");
        assert_eq!(windows.format.extension(), "zip");
        assert!(for_platform("windows", "aarch64").is_err());
        assert!(for_platform("macos", "x86_64").is_err());
        assert!(find("x86_64-apple-darwin").is_err());
        assert!(find("x86_64-pc-windows-gnu").is_err());
    }
}
