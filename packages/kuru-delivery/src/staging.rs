//! Private, exclusively owned delivery staging on the destination volume.

use anyhow::{Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use std::{ffi::OsStr, fs, path::Path};

pub(crate) struct Stage {
    directory: Option<Directory>,
}

impl Stage {
    pub(crate) fn create(parent: &Directory, prefix: &str) -> Result<Self> {
        let name = format!("{prefix}{}", uuid::Uuid::new_v4());
        Ok(Self {
            directory: Some(parent.create_private_directory(OsStr::new(&name))?),
        })
    }

    pub(crate) fn directory(&self) -> &Directory {
        self.directory
            .as_ref()
            .expect("owned stage has not finished")
    }

    pub(crate) fn path(&self) -> &Path {
        self.directory().path()
    }

    pub(crate) fn finish(mut self) -> Result<()> {
        self.remove()
    }

    #[cfg(windows)]
    pub(crate) fn keep(mut self) -> std::path::PathBuf {
        let directory = self.directory.take().expect("owned stage has not finished");
        directory.path().to_owned()
    }

    fn remove(&mut self) -> Result<()> {
        let Some(held) = &self.directory else {
            return Ok(());
        };
        let current = Directory::open(held.path(), Privacy::OwnerOnly, NameRetention::Movable)?;
        ensure!(
            current.identity() == held.identity(),
            "delivery stage was replaced; refusing cleanup"
        );
        let path = held.path().to_owned();
        drop(current);
        drop(self.directory.take());
        // The private, exclusively created root is the cleanup authority. All
        // payload handles must have closed before explicit successful cleanup.
        fs::remove_dir_all(path)?;
        Ok(())
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}
