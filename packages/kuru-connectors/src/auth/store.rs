use super::{AuthRoute, RequestCredentials, random, validate_secret};
use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{File, TryLockError},
    io::{Read, Write},
    path::{Component, PathBuf},
    time::Duration,
};

const RECORD: &str = "credentials.json";
const LOCK: &str = "credentials.lock";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub account_id: String,
    pub expires_at: u64,
    pub session_id: String,
    pub generation: u64,
    pub refresh_pending: bool,
}
impl Session {
    pub fn credentials(&self) -> RequestCredentials {
        RequestCredentials {
            route: AuthRoute::Chatgpt,
            bearer: self.access_token.clone(),
            account_id: Some(self.account_id.clone()),
            session_id: Some(self.session_id.clone()),
            generation: Some(self.generation),
        }
    }
    fn validate(&self) -> Result<()> {
        for value in [&self.access_token, &self.refresh_token, &self.id_token] {
            validate_secret(value)?;
        }
        ensure!(
            !self.account_id.is_empty()
                && self.account_id.len() <= 1024
                && !self.account_id.chars().any(char::is_control),
            "invalid stored account identity"
        );
        ensure!(
            !self.session_id.is_empty() && self.session_id.len() <= 128 && self.expires_at > 0,
            "invalid stored authentication session"
        );
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    schema_version: u32,
    pub revision: String,
    pub session: Option<Session>,
}
impl Record {
    pub fn new(session: Option<Session>) -> Result<Self> {
        Ok(Self {
            schema_version: 1,
            revision: random(32)?,
            session,
        })
    }
}

pub(super) struct Store {
    path: PathBuf,
    tool_root: PathBuf,
    #[cfg(test)]
    faults: std::sync::Arc<std::sync::Mutex<TestFaults>>,
}

#[cfg(test)]
#[derive(Default)]
struct TestFaults {
    write_target: Option<usize>,
    writes: usize,
    read_target: Option<usize>,
    reads: usize,
}
impl Store {
    pub fn new(data: PathBuf, tool_root: PathBuf) -> Result<Self> {
        for path in [&data, &tool_root] {
            ensure!(
                path.is_absolute()
                    && !path
                        .components()
                        .any(|part| matches!(part, Component::ParentDir)),
                "authentication paths must be absolute without parent components"
            );
        }
        Ok(Self {
            path: data.join("auth").join("openai"),
            tool_root,
            #[cfg(test)]
            faults: Default::default(),
        })
    }

    fn directory(&self, create: bool) -> Result<Option<Directory>> {
        let tool = Directory::open(&self.tool_root, Privacy::Inherited, NameRetention::Movable)
            .context("open authentication tool-root boundary")?;
        // Check the nearest existing ancestor before creating a missing suffix.
        // This detects native identity aliases without canonicalizing past links.
        let mut ancestor = self.path.as_path();
        loop {
            match Directory::open(ancestor, Privacy::Inherited, NameRetention::Movable) {
                Ok(directory) => {
                    ensure!(
                        !directory.is_within(&tool)?,
                        "authentication state must be outside the tool root"
                    );
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    ancestor = ancestor
                        .parent()
                        .context("authentication path has no existing ancestor")?;
                }
                Err(error) => return Err(error).context("validate authentication directory"),
            }
        }
        let directory = if create {
            Directory::ensure_private(&self.path)
        } else {
            Directory::open(&self.path, Privacy::OwnerOnly, NameRetention::Movable)
        };
        match directory {
            Ok(directory) => {
                ensure!(
                    !directory.is_within(&tool)?,
                    "authentication state must be outside the tool root"
                );
                Ok(Some(directory))
            }
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("open private authentication directory"),
        }
    }

    pub fn read(&self) -> Result<Option<Record>> {
        self.directory(false)?
            .as_ref()
            .map(read_record)
            .transpose()
            .map(Option::flatten)
    }

    pub async fn lease(&self, create: bool, timeout: Duration) -> Result<Option<Lease>> {
        let Some(directory) = self.directory(create)? else {
            return Ok(None);
        };
        let lock = directory
            .lock_file(OsStr::new(LOCK))
            .context("open private authentication lease")?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    ensure!(
                        tokio::time::Instant::now() < deadline,
                        "authentication lease deadline exceeded"
                    );
                    tokio::time::sleep_until(
                        (tokio::time::Instant::now() + Duration::from_millis(10)).min(deadline),
                    )
                    .await;
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error).context("lock private authentication store");
                }
            }
        }
        directory
            .verify(OsStr::new(LOCK), &lock)
            .context("authentication lease identity changed")?;
        Ok(Some(Lease {
            directory,
            lock,
            #[cfg(test)]
            faults: self.faults.clone(),
        }))
    }

    #[cfg(test)]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    #[cfg(test)]
    pub fn fail_after_publish_on_write(&self, target: usize) {
        let mut faults = self.faults.lock().unwrap();
        faults.write_target = Some(target);
        faults.writes = 0;
    }

    #[cfg(test)]
    pub fn fail_on_read(&self, target: usize) {
        let mut faults = self.faults.lock().unwrap();
        faults.read_target = Some(target);
        faults.reads = 0;
    }
}

pub(super) struct Lease {
    directory: Directory,
    lock: File,
    #[cfg(test)]
    faults: std::sync::Arc<std::sync::Mutex<TestFaults>>,
}
impl Lease {
    pub fn read(&self) -> Result<Option<Record>> {
        #[cfg(test)]
        {
            let mut faults = self.faults.lock().unwrap();
            faults.reads += 1;
            if faults.read_target == Some(faults.reads) {
                faults.read_target = None;
                anyhow::bail!("synthetic authentication read failure")
            }
        }
        self.directory.verify(OsStr::new(LOCK), &self.lock)?;
        read_record(&self.directory)
    }
    pub fn write(&self, record: &Record) -> Result<()> {
        self.directory.verify(OsStr::new(LOCK), &self.lock)?;
        let bytes = serde_json::to_vec(record).context("encode private authentication record")?;
        ensure!(
            bytes.len() <= crate::MAX_BYTES,
            "authentication record exceeds size limit"
        );
        let name = format!("credentials-{}.tmp", random(24)?);
        let mut file = self
            .directory
            .create_new(OsStr::new(&name))
            .context("create private authentication candidate")?;
        file.write_all(&bytes)
            .context("write private authentication candidate")?;
        self.directory
            .publish_file(
                &self.directory,
                OsStr::new(&name),
                &file,
                OsStr::new(RECORD),
                Publication::ReplaceRegular,
            )
            .context(
                "publish private authentication record; uncertain candidates remain private",
            )?;
        #[cfg(test)]
        {
            let mut faults = self.faults.lock().unwrap();
            faults.writes += 1;
            if faults.write_target == Some(faults.writes) {
                faults.write_target = None;
                anyhow::bail!("synthetic authentication publication reply loss")
            }
        }
        Ok(())
    }
}

fn read_record(directory: &Directory) -> Result<Option<Record>> {
    let mut file = match directory.read(OsStr::new(RECORD)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read private authentication record"),
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take(crate::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("read authentication bytes")?;
    ensure!(
        bytes.len() <= crate::MAX_BYTES,
        "authentication record exceeds size limit"
    );
    directory
        .verify(OsStr::new(RECORD), &file)
        .context("authentication record identity changed")?;
    // Serde errors can quote the invalid field value, so do not expose them.
    let record: Record = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid private authentication record"))?;
    ensure!(
        record.schema_version == 1 && !record.revision.is_empty() && record.revision.len() <= 128,
        "unsupported private authentication record"
    );
    if let Some(session) = &record.session {
        session.validate()?;
    }
    Ok(Some(record))
}
