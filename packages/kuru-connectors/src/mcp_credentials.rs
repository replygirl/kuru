//! Nonsecret serialization for alias-scoped native MCP credentials.

use std::{
    ffi::OsStr,
    fs::{File, TryLockError},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use kuru_core::ManifestDigest;
use kuru_platform::{
    fs::{Directory, NameRetention, Privacy},
    secret::{
        MAX_NATIVE_SECRET_BYTES, NativeSecretAccount, NativeSecretError, NativeSecretErrorKind,
        NativeSecretGeneration, NativeSecretRecord, NativeSecretStore,
    },
};
use sha2::{Digest, Sha256};

use crate::mcp_cache::ensure_outside_root;

const LOCK_DEADLINE: Duration = Duration::from_secs(30);
const LOCK_POLL: Duration = Duration::from_millis(25);
const LOGICAL_CREDENTIAL_BYTES: usize = 67_899;
const MAX_CHUNKS: usize = 27;
const MANIFEST_MAGIC: &[u8; 16] = b"KURU-MCP-CRED-v1";
const MANIFEST_VERSION: u8 = 1;
const DESCRIPTOR_BYTES: usize = 16 + 4 + 32 + 1;
const MANIFEST_HEADER_BYTES: usize = MANIFEST_MAGIC.len() + 2;
const MAX_MANIFEST_BYTES: usize = MANIFEST_HEADER_BYTES + 2 * DESCRIPTOR_BYTES;
const CREDENTIAL_MAGIC: &[u8; 16] = b"KURU-MCP-OAUTH\0\0";
const CREDENTIAL_VERSION: u8 = 1;
const MAX_CLIENT_ID_BYTES: usize = 2_048;
const MAX_SCOPE_COUNT: usize = 64;
const MAX_SCOPE_BYTES: usize = 256;
const MAX_SECRET_BYTES: usize = 16 * 1_024;

/// App-injected private storage for nonsecret cross-process serialization.
/// Credential bytes remain exclusively in the operating-system secret store.
pub struct McpCredentialStore {
    data: PathBuf,
    root: Arc<Directory>,
    project_identity: [u8; 24],
    authority: [u8; 32],
    directory_name: String,
}

impl McpCredentialStore {
    pub fn new(data: &Path, root: Arc<Directory>, authority: ManifestDigest) -> Result<Self> {
        root.revalidate()?;
        match Directory::open(data, Privacy::OwnerOnly, NameRetention::Movable) {
            Ok(existing) => ensure_outside_root(&existing, &root)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("open private MCP credential data directory"),
        }
        let digest = Sha256::digest(root.path().as_os_str().as_encoded_bytes());
        Ok(Self {
            data: data.to_owned(),
            project_identity: root.identity().to_bytes(),
            root,
            authority: authority.as_bytes(),
            directory_name: format!("mcp-oauth-{}", hex(&digest)),
        })
    }

    pub(crate) fn binding_authority(&self) -> [u8; 32] {
        self.authority
    }

    pub(crate) fn project_identity(&self) -> [u8; 24] {
        self.project_identity
    }

    pub(crate) async fn acquire(&self, alias: &str) -> Result<McpCredentialLease> {
        self.check_root()?;
        let directory = self.create_directory()?;
        let digest = account_digest(self.project_identity, alias);
        let account = NativeSecretAccount::from_digest(digest);
        let name = format!("{}.lock", hex(&digest));
        let file = directory.lock_file(OsStr::new(&name))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if started.elapsed() < LOCK_DEADLINE => {
                    tokio::time::sleep(LOCK_POLL).await;
                }
                Err(TryLockError::WouldBlock) => {
                    anyhow::bail!("MCP credential operation is busy")
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error).context("lock MCP credential operation");
                }
            }
        }
        directory.verify(OsStr::new(&name), &file)?;
        self.check_root()?;
        Ok(McpCredentialLease {
            _lock: file,
            manifest_account: account,
            account_seed: digest,
        })
    }

    fn check_root(&self) -> Result<()> {
        self.root.revalidate()?;
        ensure!(
            self.root.identity().to_bytes() == self.project_identity,
            "MCP credential workspace changed"
        );
        Ok(())
    }

    fn create_directory(&self) -> Result<Directory> {
        let data = Directory::ensure_private(&self.data)
            .context("MCP credential data directory is not owner-private")?;
        ensure_outside_root(&data, &self.root)?;
        let name = OsStr::new(&self.directory_name);
        match data.create_private_directory(name) {
            Ok(directory) => Ok(directory),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Directory::open(
                &data.path().join(name),
                Privacy::OwnerOnly,
                NameRetention::Movable,
            )
            .context("open private MCP credential directory"),
            Err(error) => Err(error).context("create private MCP credential directory"),
        }
    }
}

pub(crate) struct McpCredentialLease {
    _lock: File,
    manifest_account: NativeSecretAccount,
    account_seed: [u8; 32],
}

impl McpCredentialLease {
    pub(crate) async fn get(self) -> Result<Option<McpCredentialRecord>> {
        let (_, record) = self.read_locked().await?;
        Ok(record)
    }

    /// Read while returning the same alias lock to a multi-step login, refresh,
    /// or logout flow. If the caller cancels during the native operation, the
    /// blocking worker retains and eventually drops the lock itself.
    pub(crate) async fn read_locked(self) -> Result<(Self, Option<McpCredentialRecord>)> {
        let Self {
            _lock,
            manifest_account,
            account_seed,
        } = self;
        let (lock, manifest_account, record) = tokio::task::spawn_blocking(move || {
            let result = Vault::new(
                NativeSecretStore::new(),
                manifest_account.clone(),
                account_seed,
            )
            .read();
            (_lock, manifest_account, result)
        })
        .await
        .context("native secret operation worker stopped")?;
        Ok((
            Self {
                _lock: lock,
                manifest_account,
                account_seed,
            },
            record?,
        ))
    }

    pub(crate) async fn create(self, secret: Vec<u8>) -> Result<McpCredentialGeneration> {
        let Self {
            _lock,
            manifest_account,
            account_seed,
        } = self;
        native(move |store| {
            let _lock = _lock;
            Vault::new(store, manifest_account, account_seed).create(secret)
        })
        .await
    }

    pub(crate) async fn replace(
        self,
        expected: McpCredentialGeneration,
        secret: Vec<u8>,
    ) -> Result<McpCredentialGeneration> {
        let Self {
            _lock,
            manifest_account,
            account_seed,
        } = self;
        native(move |store| {
            let _lock = _lock;
            Vault::new(store, manifest_account, account_seed).replace(expected, secret)
        })
        .await
    }

    pub(crate) async fn delete(self, expected: McpCredentialGeneration) -> Result<()> {
        let Self {
            _lock,
            manifest_account,
            account_seed,
        } = self;
        native(move |store| {
            let _lock = _lock;
            Vault::new(store, manifest_account, account_seed).delete(expected)
        })
        .await
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct McpCredentialGeneration([u8; 16]);

impl McpCredentialGeneration {
    pub(crate) const fn bytes(self) -> [u8; 16] {
        self.0
    }
}

impl std::fmt::Debug for McpCredentialGeneration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("McpCredentialGeneration([opaque])")
    }
}

pub(crate) struct McpCredentialRecord {
    generation: McpCredentialGeneration,
    secret: Vec<u8>,
}

impl McpCredentialRecord {
    pub(crate) const fn generation(&self) -> McpCredentialGeneration {
        self.generation
    }

    pub(crate) fn secret(&self) -> &[u8] {
        &self.secret
    }
}

impl std::fmt::Debug for McpCredentialRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpCredentialRecord")
            .field("generation", &self.generation)
            .field("secret", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum McpRegistrationKind {
    Configured = 1,
    ClientMetadata = 2,
    Dynamic = 3,
}

/// Secret-bearing logical record. Debug and serialization are deliberately
/// absent; the compact codec is the only durable representation.
pub(crate) struct McpOAuthCredential {
    pub(crate) authority: [u8; 32],
    pub(crate) project: [u8; 24],
    pub(crate) alias: [u8; 32],
    pub(crate) resource: [u8; 32],
    pub(crate) issuer: [u8; 32],
    pub(crate) registration: McpRegistrationKind,
    pub(crate) expires_at: u64,
    pub(crate) client_id: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) client_secret: Option<String>,
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
}

impl McpOAuthCredential {
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        validate_text("client ID", &self.client_id, 1, MAX_CLIENT_ID_BYTES)?;
        ensure!(
            self.scopes.len() <= MAX_SCOPE_COUNT,
            "too many MCP OAuth credential scopes"
        );
        let mut prior = None;
        for scope in &self.scopes {
            validate_text("scope", scope, 1, MAX_SCOPE_BYTES)?;
            ensure!(
                prior.is_none_or(|prior: &String| prior < scope),
                "MCP OAuth credential scopes must be unique and sorted"
            );
            prior = Some(scope);
        }
        validate_optional_secret("client secret", self.client_secret.as_deref())?;
        validate_text("access token", &self.access_token, 1, MAX_SECRET_BYTES)?;
        validate_optional_secret("refresh token", self.refresh_token.as_deref())?;

        let mut bytes = Vec::with_capacity(LOGICAL_CREDENTIAL_BYTES);
        bytes.extend_from_slice(CREDENTIAL_MAGIC);
        bytes.push(CREDENTIAL_VERSION);
        bytes.extend_from_slice(&self.authority);
        bytes.extend_from_slice(&self.project);
        bytes.extend_from_slice(&self.alias);
        bytes.extend_from_slice(&self.resource);
        bytes.extend_from_slice(&self.issuer);
        bytes.push(self.registration as u8);
        bytes.extend_from_slice(&self.expires_at.to_be_bytes());
        encode_text(&mut bytes, &self.client_id)?;
        bytes.push(u8::try_from(self.scopes.len())?);
        for scope in &self.scopes {
            encode_text(&mut bytes, scope)?;
        }
        encode_text(
            &mut bytes,
            self.client_secret.as_deref().unwrap_or_default(),
        )?;
        encode_text(&mut bytes, &self.access_token)?;
        encode_text(
            &mut bytes,
            self.refresh_token.as_deref().unwrap_or_default(),
        )?;
        ensure!(
            bytes.len() <= LOGICAL_CREDENTIAL_BYTES,
            "MCP OAuth credential exceeds its byte bound"
        );
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= LOGICAL_CREDENTIAL_BYTES,
            "MCP OAuth credential exceeds its byte bound"
        );
        let mut cursor = CredentialCursor::new(bytes);
        ensure!(
            cursor.take(16)? == CREDENTIAL_MAGIC,
            "MCP OAuth credential magic is invalid"
        );
        ensure!(
            cursor.byte()? == CREDENTIAL_VERSION,
            "MCP OAuth credential version is unsupported"
        );
        let authority = cursor.array()?;
        let project = cursor.array()?;
        let alias = cursor.array()?;
        let resource = cursor.array()?;
        let issuer = cursor.array()?;
        let registration = match cursor.byte()? {
            1 => McpRegistrationKind::Configured,
            2 => McpRegistrationKind::ClientMetadata,
            3 => McpRegistrationKind::Dynamic,
            _ => anyhow::bail!("MCP OAuth credential registration kind is invalid"),
        };
        let expires_at = u64::from_be_bytes(cursor.array()?);
        let client_id = cursor.text("client ID", 1, MAX_CLIENT_ID_BYTES)?;
        let scope_count = usize::from(cursor.byte()?);
        ensure!(
            scope_count <= MAX_SCOPE_COUNT,
            "too many MCP OAuth credential scopes"
        );
        let mut scopes = Vec::with_capacity(scope_count);
        for _ in 0..scope_count {
            scopes.push(cursor.text("scope", 1, MAX_SCOPE_BYTES)?);
        }
        let client_secret = cursor.optional_text("client secret", MAX_SECRET_BYTES)?;
        let access_token = cursor.text("access token", 1, MAX_SECRET_BYTES)?;
        let refresh_token = cursor.optional_text("refresh token", MAX_SECRET_BYTES)?;
        ensure!(
            cursor.remaining().is_empty(),
            "MCP OAuth credential has trailing bytes"
        );
        let credential = Self {
            authority,
            project,
            alias,
            resource,
            issuer,
            registration,
            expires_at,
            client_id,
            scopes,
            client_secret,
            access_token,
            refresh_token,
        };
        ensure!(
            credential.encode()? == bytes,
            "MCP OAuth credential encoding is not canonical"
        );
        Ok(credential)
    }
}

struct CredentialCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CredentialCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .context("MCP OAuth credential length overflow")?;
        let value = self
            .bytes
            .get(self.offset..end)
            .context("MCP OAuth credential is truncated")?;
        self.offset = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into()?)
    }

    fn text(&mut self, kind: &str, minimum: usize, maximum: usize) -> Result<String> {
        let length = usize::from(u16::from_be_bytes(self.array()?));
        ensure!(
            (minimum..=maximum).contains(&length),
            "MCP OAuth credential {kind} exceeds its bound"
        );
        let value = std::str::from_utf8(self.take(length)?)
            .with_context(|| format!("MCP OAuth credential {kind} is not UTF-8"))?
            .to_owned();
        Ok(value)
    }

    fn optional_text(&mut self, kind: &str, maximum: usize) -> Result<Option<String>> {
        let length = usize::from(u16::from_be_bytes(self.array()?));
        ensure!(
            length <= maximum,
            "MCP OAuth credential {kind} exceeds its bound"
        );
        if length == 0 {
            return Ok(None);
        }
        Ok(Some(
            std::str::from_utf8(self.take(length)?)
                .with_context(|| format!("MCP OAuth credential {kind} is not UTF-8"))?
                .to_owned(),
        ))
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
}

fn validate_text(kind: &str, value: &str, minimum: usize, maximum: usize) -> Result<()> {
    ensure!(
        (minimum..=maximum).contains(&value.len()),
        "MCP OAuth credential {kind} exceeds its bound"
    );
    Ok(())
}

fn validate_optional_secret(kind: &str, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        validate_text(kind, value, 1, MAX_SECRET_BYTES)?;
    }
    Ok(())
}

fn encode_text(bytes: &mut Vec<u8>, value: &str) -> Result<()> {
    bytes.extend_from_slice(&u16::try_from(value.len())?.to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemError {
    Stale,
    Denied,
    Unavailable,
    Corrupt,
}

trait ItemStore {
    fn get(
        &mut self,
        account: &NativeSecretAccount,
    ) -> std::result::Result<Option<NativeSecretRecord>, ItemError>;
    fn create(
        &mut self,
        account: &NativeSecretAccount,
        record: &NativeSecretRecord,
    ) -> std::result::Result<(), ItemError>;
    fn replace(
        &mut self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
        record: &NativeSecretRecord,
    ) -> std::result::Result<(), ItemError>;
    fn delete(
        &mut self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
    ) -> std::result::Result<(), ItemError>;
}

impl ItemStore for NativeSecretStore {
    fn get(
        &mut self,
        account: &NativeSecretAccount,
    ) -> std::result::Result<Option<NativeSecretRecord>, ItemError> {
        match NativeSecretStore::get(self, account) {
            Ok(record) => Ok(Some(record)),
            Err(error) if error.kind() == NativeSecretErrorKind::NotFound => Ok(None),
            Err(error) => Err(map_item_error(&error)),
        }
    }

    fn create(
        &mut self,
        account: &NativeSecretAccount,
        record: &NativeSecretRecord,
    ) -> std::result::Result<(), ItemError> {
        NativeSecretStore::create(self, account, record).map_err(|error| map_item_error(&error))
    }

    fn replace(
        &mut self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
        record: &NativeSecretRecord,
    ) -> std::result::Result<(), ItemError> {
        NativeSecretStore::replace(self, account, expected, record)
            .map_err(|error| map_item_error(&error))
    }

    fn delete(
        &mut self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
    ) -> std::result::Result<(), ItemError> {
        NativeSecretStore::delete(self, account, expected).map_err(|error| map_item_error(&error))
    }
}

fn map_item_error(error: &NativeSecretError) -> ItemError {
    match error.kind() {
        NativeSecretErrorKind::NotFound | NativeSecretErrorKind::Stale => ItemError::Stale,
        NativeSecretErrorKind::Denied => ItemError::Denied,
        NativeSecretErrorKind::Unavailable => ItemError::Unavailable,
        NativeSecretErrorKind::Corrupt => ItemError::Corrupt,
    }
}

#[derive(Clone, Eq, PartialEq)]
struct Descriptor {
    generation: McpCredentialGeneration,
    length: u32,
    digest: [u8; 32],
    chunks: u8,
}

#[derive(Clone, Eq, PartialEq)]
enum Manifest {
    Stable(Descriptor),
    PreparedCreate {
        new: Descriptor,
    },
    PreparedReplace {
        old: Descriptor,
        new: Descriptor,
    },
    CommittedNew {
        new: Descriptor,
        retired: Descriptor,
    },
    CommittedDelete {
        retired: Descriptor,
    },
}

struct LoadedManifest {
    native_generation: NativeSecretGeneration,
    state: Manifest,
}

struct Vault<S> {
    store: S,
    manifest_account: NativeSecretAccount,
    account_seed: [u8; 32],
}

impl<S: ItemStore> Vault<S> {
    fn new(store: S, manifest_account: NativeSecretAccount, account_seed: [u8; 32]) -> Self {
        Self {
            store,
            manifest_account,
            account_seed,
        }
    }

    fn read(&mut self) -> Result<Option<McpCredentialRecord>> {
        let Some(manifest) = self.recover()? else {
            return Ok(None);
        };
        let Manifest::Stable(descriptor) = manifest.state else {
            unreachable!("recovery returns stable state");
        };
        let secret = self
            .try_read_chunks(&descriptor)?
            .context("MCP OAuth credential chunks are incomplete")?;
        Ok(Some(McpCredentialRecord {
            generation: descriptor.generation,
            secret,
        }))
    }

    fn create(&mut self, secret: Vec<u8>) -> Result<McpCredentialGeneration> {
        ensure!(
            self.recover()?.is_none(),
            "MCP OAuth credential generation changed"
        );
        let descriptor = descriptor(&secret)?;
        let prepared = Manifest::PreparedCreate {
            new: descriptor.clone(),
        };
        let native_generation = native_generation()?;
        let record = manifest_record(native_generation, &prepared)?;
        self.store
            .create(&self.manifest_account, &record)
            .map_err(item_failure)?;
        let loaded = LoadedManifest {
            native_generation,
            state: prepared,
        };
        self.write_chunks(&descriptor, &secret)?;
        let loaded = self.transition(loaded, Manifest::Stable(descriptor.clone()))?;
        debug_assert!(matches!(loaded.state, Manifest::Stable(_)));
        Ok(descriptor.generation)
    }

    fn replace(
        &mut self,
        expected: McpCredentialGeneration,
        secret: Vec<u8>,
    ) -> Result<McpCredentialGeneration> {
        let current = self
            .recover()?
            .context("MCP OAuth credential does not exist")?;
        let Manifest::Stable(old) = current.state.clone() else {
            unreachable!("recovery returns stable state");
        };
        ensure!(
            old.generation == expected,
            "MCP OAuth credential generation changed"
        );
        let new = descriptor(&secret)?;
        ensure!(
            new.generation != old.generation,
            "MCP OAuth credential generation changed"
        );
        let prepared = self.transition(
            current,
            Manifest::PreparedReplace {
                old: old.clone(),
                new: new.clone(),
            },
        )?;
        self.write_chunks(&new, &secret)?;
        let committed = self.transition(
            prepared,
            Manifest::CommittedNew {
                new: new.clone(),
                retired: old.clone(),
            },
        )?;
        self.remove_chunks(&old)?;
        let stable = self.transition(committed, Manifest::Stable(new.clone()))?;
        debug_assert!(matches!(stable.state, Manifest::Stable(_)));
        Ok(new.generation)
    }

    fn delete(&mut self, expected: McpCredentialGeneration) -> Result<()> {
        let current = self
            .recover()?
            .context("MCP OAuth credential does not exist")?;
        let Manifest::Stable(retired) = current.state.clone() else {
            unreachable!("recovery returns stable state");
        };
        ensure!(
            retired.generation == expected,
            "MCP OAuth credential generation changed"
        );
        let committed = self.transition(
            current,
            Manifest::CommittedDelete {
                retired: retired.clone(),
            },
        )?;
        self.remove_chunks(&retired)?;
        self.store
            .delete(&self.manifest_account, committed.native_generation)
            .map_err(item_failure)
    }

    fn recover(&mut self) -> Result<Option<LoadedManifest>> {
        let Some(record) = self
            .store
            .get(&self.manifest_account)
            .map_err(item_failure)?
        else {
            return Ok(None);
        };
        let mut loaded = LoadedManifest {
            native_generation: record.generation(),
            state: decode_manifest(record.secret())?,
        };
        loop {
            loaded = match loaded.state.clone() {
                Manifest::Stable(_) => return Ok(Some(loaded)),
                Manifest::PreparedCreate { new } => {
                    if self.try_read_chunks(&new)?.is_some() {
                        self.transition(loaded, Manifest::Stable(new))?
                    } else {
                        self.remove_tentative_chunks(&new)?;
                        self.store
                            .delete(&self.manifest_account, loaded.native_generation)
                            .map_err(item_failure)?;
                        return Ok(None);
                    }
                }
                Manifest::PreparedReplace { old, new } => {
                    let _ = self
                        .try_read_chunks(&old)?
                        .context("MCP OAuth rollback credential is incomplete")?;
                    if self.try_read_chunks(&new)?.is_some() {
                        self.transition(loaded, Manifest::CommittedNew { new, retired: old })?
                    } else {
                        self.remove_tentative_chunks(&new)?;
                        self.transition(loaded, Manifest::Stable(old))?
                    }
                }
                Manifest::CommittedNew { new, retired } => {
                    let _ = self
                        .try_read_chunks(&new)?
                        .context("committed MCP OAuth credential is incomplete")?;
                    self.remove_chunks(&retired)?;
                    self.transition(loaded, Manifest::Stable(new))?
                }
                Manifest::CommittedDelete { retired } => {
                    self.remove_chunks(&retired)?;
                    self.store
                        .delete(&self.manifest_account, loaded.native_generation)
                        .map_err(item_failure)?;
                    return Ok(None);
                }
            };
        }
    }

    fn transition(&mut self, prior: LoadedManifest, state: Manifest) -> Result<LoadedManifest> {
        let native_generation = native_generation()?;
        let record = manifest_record(native_generation, &state)?;
        self.store
            .replace(&self.manifest_account, prior.native_generation, &record)
            .map_err(item_failure)?;
        Ok(LoadedManifest {
            native_generation,
            state,
        })
    }

    fn write_chunks(&mut self, descriptor: &Descriptor, secret: &[u8]) -> Result<()> {
        for (index, chunk) in secret.chunks(MAX_NATIVE_SECRET_BYTES).enumerate() {
            let account = chunk_account(self.account_seed, descriptor.generation, index as u8);
            let generation = native_secret_generation(descriptor.generation)?;
            let record = NativeSecretRecord::new(generation, chunk.to_vec())
                .map_err(|_| anyhow::anyhow!("MCP OAuth credential chunk is invalid"))?;
            match self.store.get(&account).map_err(item_failure)? {
                None => self.store.create(&account, &record).map_err(item_failure)?,
                Some(existing) => ensure!(existing == record, "MCP OAuth credential chunk changed"),
            }
        }
        ensure!(
            self.try_read_chunks(descriptor)?.as_deref() == Some(secret),
            "MCP OAuth credential chunk verification failed"
        );
        Ok(())
    }

    fn try_read_chunks(&mut self, descriptor: &Descriptor) -> Result<Option<Vec<u8>>> {
        let mut secret = Vec::with_capacity(descriptor.length as usize);
        let generation = native_secret_generation(descriptor.generation)?;
        for index in 0..descriptor.chunks {
            let account = chunk_account(self.account_seed, descriptor.generation, index);
            let Some(record) = self.store.get(&account).map_err(item_failure)? else {
                return Ok(None);
            };
            if record.generation() != generation {
                return Ok(None);
            }
            secret.extend_from_slice(record.secret());
            if secret.len() > descriptor.length as usize {
                return Ok(None);
            }
        }
        if secret.len() != descriptor.length as usize
            || Sha256::digest(&secret).as_slice() != descriptor.digest
        {
            return Ok(None);
        }
        Ok(Some(secret))
    }

    fn remove_chunks(&mut self, descriptor: &Descriptor) -> Result<()> {
        let expected = native_secret_generation(descriptor.generation)?;
        for index in 0..descriptor.chunks {
            let account = chunk_account(self.account_seed, descriptor.generation, index);
            if let Some(record) = self.store.get(&account).map_err(item_failure)? {
                ensure!(
                    record.generation() == expected,
                    "MCP OAuth retired chunk generation changed"
                );
                self.store
                    .delete(&account, expected)
                    .map_err(item_failure)?;
            }
        }
        Ok(())
    }

    /// Roll back a generation that never became authoritative. Exact owned
    /// chunks are removed, while a deterministic-account collision is left
    /// untouched so an unrelated native record cannot strand the manifest in
    /// its prepared state. Committed retirement deliberately uses the strict
    /// `remove_chunks` path above.
    fn remove_tentative_chunks(&mut self, descriptor: &Descriptor) -> Result<()> {
        let expected = native_secret_generation(descriptor.generation)?;
        for index in 0..descriptor.chunks {
            let account = chunk_account(self.account_seed, descriptor.generation, index);
            let Some(record) = self.store.get(&account).map_err(item_failure)? else {
                continue;
            };
            if record.generation() != expected {
                continue;
            }
            match self.store.delete(&account, expected) {
                Ok(()) | Err(ItemError::Stale) => {}
                Err(error) => return Err(item_failure(error)),
            }
        }
        Ok(())
    }
}

fn descriptor(secret: &[u8]) -> Result<Descriptor> {
    ensure!(
        !secret.is_empty() && secret.len() <= LOGICAL_CREDENTIAL_BYTES,
        "MCP OAuth credential exceeds its byte bound"
    );
    let chunks = secret.len().div_ceil(MAX_NATIVE_SECRET_BYTES);
    ensure!(
        (1..=MAX_CHUNKS).contains(&chunks),
        "MCP OAuth credential chunk count exceeds its bound"
    );
    Ok(Descriptor {
        generation: McpCredentialGeneration(random_generation()?),
        length: u32::try_from(secret.len())?,
        digest: Sha256::digest(secret).into(),
        chunks: u8::try_from(chunks)?,
    })
}

fn native_generation() -> Result<NativeSecretGeneration> {
    NativeSecretGeneration::new(random_generation()?)
        .map_err(|_| anyhow::anyhow!("native credential generation is invalid"))
}

fn native_secret_generation(generation: McpCredentialGeneration) -> Result<NativeSecretGeneration> {
    NativeSecretGeneration::new(generation.0)
        .map_err(|_| anyhow::anyhow!("MCP OAuth credential generation is invalid"))
}

fn random_generation() -> Result<[u8; 16]> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).context("operating system randomness is unavailable")?;
    ensure!(
        bytes != [0; 16],
        "operating system returned an invalid generation"
    );
    Ok(bytes)
}

fn chunk_account(
    seed: [u8; 32],
    generation: McpCredentialGeneration,
    index: u8,
) -> NativeSecretAccount {
    let mut digest = Sha256::new();
    digest.update(b"kuru.mcp.oauth.chunk.v1\0");
    digest.update(seed);
    digest.update(generation.0);
    digest.update([index]);
    NativeSecretAccount::from_digest(digest.finalize().into())
}

fn manifest_record(
    generation: NativeSecretGeneration,
    manifest: &Manifest,
) -> Result<NativeSecretRecord> {
    NativeSecretRecord::new(generation, encode_manifest(manifest))
        .map_err(|_| anyhow::anyhow!("MCP OAuth credential manifest is invalid"))
}

fn encode_manifest(manifest: &Manifest) -> Vec<u8> {
    let (state, descriptors): (u8, &[&Descriptor]) = match manifest {
        Manifest::Stable(current) => (1, &[current]),
        Manifest::PreparedCreate { new } => (2, &[new]),
        Manifest::PreparedReplace { old, new } => (3, &[old, new]),
        Manifest::CommittedNew { new, retired } => (4, &[new, retired]),
        Manifest::CommittedDelete { retired } => (5, &[retired]),
    };
    let mut bytes =
        Vec::with_capacity(MANIFEST_HEADER_BYTES + descriptors.len() * DESCRIPTOR_BYTES);
    bytes.extend_from_slice(MANIFEST_MAGIC);
    bytes.push(MANIFEST_VERSION);
    bytes.push(state);
    for descriptor in descriptors {
        bytes.extend_from_slice(&descriptor.generation.0);
        bytes.extend_from_slice(&descriptor.length.to_be_bytes());
        bytes.extend_from_slice(&descriptor.digest);
        bytes.push(descriptor.chunks);
    }
    bytes
}

fn decode_manifest(bytes: &[u8]) -> Result<Manifest> {
    ensure!(
        bytes.len() >= MANIFEST_HEADER_BYTES && &bytes[..16] == MANIFEST_MAGIC,
        "MCP OAuth credential manifest is invalid"
    );
    ensure!(
        bytes[16] == MANIFEST_VERSION,
        "MCP OAuth credential manifest version is unsupported"
    );
    let state = bytes[17];
    let expected = if matches!(state, 3 | 4) {
        MAX_MANIFEST_BYTES
    } else {
        MANIFEST_HEADER_BYTES + DESCRIPTOR_BYTES
    };
    ensure!(
        bytes.len() == expected,
        "MCP OAuth credential manifest length is invalid"
    );
    let first =
        decode_descriptor(&bytes[MANIFEST_HEADER_BYTES..MANIFEST_HEADER_BYTES + DESCRIPTOR_BYTES])?;
    let second = (expected == MAX_MANIFEST_BYTES)
        .then(|| decode_descriptor(&bytes[MANIFEST_HEADER_BYTES + DESCRIPTOR_BYTES..]))
        .transpose()?;
    match (state, second) {
        (1, None) => Ok(Manifest::Stable(first)),
        (2, None) => Ok(Manifest::PreparedCreate { new: first }),
        (3, Some(new)) => Ok(Manifest::PreparedReplace { old: first, new }),
        (4, Some(retired)) => Ok(Manifest::CommittedNew {
            new: first,
            retired,
        }),
        (5, None) => Ok(Manifest::CommittedDelete { retired: first }),
        _ => anyhow::bail!("MCP OAuth credential manifest state is invalid"),
    }
}

fn decode_descriptor(bytes: &[u8]) -> Result<Descriptor> {
    ensure!(
        bytes.len() == DESCRIPTOR_BYTES,
        "MCP OAuth credential descriptor is invalid"
    );
    let generation: [u8; 16] = bytes[..16].try_into()?;
    ensure!(
        generation != [0; 16],
        "MCP OAuth credential generation is invalid"
    );
    let length = u32::from_be_bytes(bytes[16..20].try_into()?);
    let digest = bytes[20..52].try_into()?;
    let chunks = bytes[52];
    ensure!(
        (1..=MAX_CHUNKS as u8).contains(&chunks),
        "MCP OAuth credential chunk count is invalid"
    );
    ensure!(
        (1..=LOGICAL_CREDENTIAL_BYTES as u32).contains(&length),
        "MCP OAuth credential length is invalid"
    );
    ensure!(
        usize::from(chunks) == (length as usize).div_ceil(MAX_NATIVE_SECRET_BYTES),
        "MCP OAuth credential chunk count does not match its length"
    );
    Ok(Descriptor {
        generation: McpCredentialGeneration(generation),
        length,
        digest,
        chunks,
    })
}

fn item_failure(error: ItemError) -> anyhow::Error {
    anyhow::anyhow!(match error {
        ItemError::Stale => "MCP OAuth native credential changed",
        ItemError::Denied => "native secret store denied the MCP OAuth credential operation",
        ItemError::Unavailable => "native secret store is unavailable for MCP OAuth credentials",
        ItemError::Corrupt => "native MCP OAuth credential record is invalid",
    })
}

async fn native<T: Send + 'static>(
    operation: impl FnOnce(NativeSecretStore) -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || operation(NativeSecretStore::new()))
        .await
        .context("native secret operation worker stopped")?
}

fn account_digest(project_identity: [u8; 24], alias: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"kuru.mcp.oauth.account.v1\0");
    digest.update(project_identity);
    digest.update((alias.len() as u64).to_be_bytes());
    digest.update(alias.as_bytes());
    digest.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::{ConfigSnapshot, InvocationOverrides};
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct SyntheticItems {
        records: Arc<Mutex<Vec<(NativeSecretAccount, NativeSecretRecord)>>>,
        failure: Arc<Mutex<Option<(ItemOperation, ItemError)>>>,
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum ItemOperation {
        Get,
        Create,
        Replace,
        Delete,
    }

    impl SyntheticItems {
        fn count(&self) -> usize {
            self.records.lock().unwrap().len()
        }

        fn fail(&self, operation: ItemOperation, error: ItemError) {
            *self.failure.lock().unwrap() = Some((operation, error));
        }

        fn check(&self, operation: ItemOperation) -> std::result::Result<(), ItemError> {
            match *self.failure.lock().unwrap() {
                Some((selected, error)) if selected == operation => Err(error),
                _ => Ok(()),
            }
        }
    }

    impl ItemStore for SyntheticItems {
        fn get(
            &mut self,
            account: &NativeSecretAccount,
        ) -> std::result::Result<Option<NativeSecretRecord>, ItemError> {
            self.check(ItemOperation::Get)?;
            Ok(self
                .records
                .lock()
                .unwrap()
                .iter()
                .find(|(candidate, _)| candidate == account)
                .map(|(_, record)| record.clone()))
        }

        fn create(
            &mut self,
            account: &NativeSecretAccount,
            record: &NativeSecretRecord,
        ) -> std::result::Result<(), ItemError> {
            self.check(ItemOperation::Create)?;
            let mut records = self.records.lock().unwrap();
            if records.iter().any(|(candidate, _)| candidate == account) {
                return Err(ItemError::Stale);
            }
            records.push((account.clone(), record.clone()));
            Ok(())
        }

        fn replace(
            &mut self,
            account: &NativeSecretAccount,
            expected: NativeSecretGeneration,
            record: &NativeSecretRecord,
        ) -> std::result::Result<(), ItemError> {
            self.check(ItemOperation::Replace)?;
            let mut records = self.records.lock().unwrap();
            let (_, current) = records
                .iter_mut()
                .find(|(candidate, _)| candidate == account)
                .ok_or(ItemError::Stale)?;
            if current.generation() != expected || record.generation() == expected {
                return Err(ItemError::Stale);
            }
            *current = record.clone();
            Ok(())
        }

        fn delete(
            &mut self,
            account: &NativeSecretAccount,
            expected: NativeSecretGeneration,
        ) -> std::result::Result<(), ItemError> {
            self.check(ItemOperation::Delete)?;
            let mut records = self.records.lock().unwrap();
            let position = records
                .iter()
                .position(|(candidate, record)| {
                    candidate == account && record.generation() == expected
                })
                .ok_or(ItemError::Stale)?;
            records.remove(position);
            Ok(())
        }
    }

    fn vault(
        store: SyntheticItems,
        manifest: &NativeSecretAccount,
        seed: [u8; 32],
    ) -> Vault<SyntheticItems> {
        Vault::new(store, manifest.clone(), seed)
    }

    #[test]
    fn native_store_denial_or_unavailability_never_falls_back_to_plaintext() {
        const SECRET: &[u8] = b"synthetic-private-token";
        for (failure, message) in [
            (
                ItemError::Denied,
                "native secret store denied the MCP OAuth credential operation",
            ),
            (
                ItemError::Unavailable,
                "native secret store is unavailable for MCP OAuth credentials",
            ),
        ] {
            // Denied models a locked or refused backend when the platform
            // reports NoStorageAccess. OS classification remains separate;
            // there is no file or session fallback here.
            for operation in [
                ItemOperation::Get,
                ItemOperation::Create,
                ItemOperation::Replace,
                ItemOperation::Delete,
            ] {
                let items = SyntheticItems::default();
                let seed = [0x51; 32];
                let manifest = NativeSecretAccount::from_digest(seed);
                let prior = if operation == ItemOperation::Replace {
                    Some(
                        vault(items.clone(), &manifest, seed)
                            .create(SECRET.to_vec())
                            .unwrap(),
                    )
                } else {
                    None
                };
                if operation == ItemOperation::Delete {
                    let pending = Manifest::PreparedCreate {
                        new: descriptor(SECRET).unwrap(),
                    };
                    let record = manifest_record(native_generation().unwrap(), &pending).unwrap();
                    items.clone().create(&manifest, &record).unwrap();
                }
                let before = items.records.lock().unwrap().clone();
                items.fail(operation, failure);
                let result: Result<()> = match operation {
                    ItemOperation::Get | ItemOperation::Delete => {
                        vault(items.clone(), &manifest, seed).read().map(|_| ())
                    }
                    ItemOperation::Create => vault(items.clone(), &manifest, seed)
                        .create(SECRET.to_vec())
                        .map(|_| ()),
                    ItemOperation::Replace => vault(items.clone(), &manifest, seed)
                        .replace(prior.unwrap(), b"replacement-private-token".to_vec())
                        .map(|_| ()),
                };
                let diagnostic = result.unwrap_err().to_string();
                assert_eq!(diagnostic, message);
                assert!(!diagnostic.contains("private-token"));
                assert!(items.records.lock().unwrap().as_slice() == before.as_slice());
                *items.failure.lock().unwrap() = None;
                let readable = vault(items.clone(), &manifest, seed).read().unwrap();
                if operation == ItemOperation::Replace {
                    assert_eq!(readable.unwrap().secret(), SECRET);
                } else {
                    assert!(readable.is_none());
                }
            }
        }
    }

    fn maximum_credential() -> McpOAuthCredential {
        let scopes = (0..MAX_SCOPE_COUNT)
            .map(|index| {
                let prefix = format!("scope-{index:02}-");
                format!("{prefix}{}", "s".repeat(MAX_SCOPE_BYTES - prefix.len()))
            })
            .collect();
        McpOAuthCredential {
            authority: [1; 32],
            project: [2; 24],
            alias: [3; 32],
            resource: [4; 32],
            issuer: [5; 32],
            registration: McpRegistrationKind::Dynamic,
            expires_at: u64::MAX,
            client_id: "c".repeat(MAX_CLIENT_ID_BYTES),
            scopes,
            client_secret: Some("s".repeat(MAX_SECRET_BYTES)),
            access_token: "a".repeat(MAX_SECRET_BYTES),
            refresh_token: Some("r".repeat(MAX_SECRET_BYTES)),
        }
    }

    #[test]
    fn logical_codec_admits_the_exact_protocol_envelope_and_rejects_noncanonical_input() {
        let credential = maximum_credential();
        let encoded = credential.encode().unwrap();
        assert_eq!(encoded.len(), LOGICAL_CREDENTIAL_BYTES);
        let decoded = McpOAuthCredential::decode(&encoded).unwrap();
        assert_eq!(decoded.client_id.len(), MAX_CLIENT_ID_BYTES);
        assert_eq!(decoded.scopes.len(), MAX_SCOPE_COUNT);
        assert_eq!(decoded.access_token.len(), MAX_SECRET_BYTES);
        assert_eq!(decoded.refresh_token.unwrap().len(), MAX_SECRET_BYTES);

        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(McpOAuthCredential::decode(&trailing).is_err());
        let mut bad_registration = encoded;
        bad_registration[169] = 0;
        assert!(McpOAuthCredential::decode(&bad_registration).is_err());

        let mut unsorted = maximum_credential();
        unsorted.scopes.swap(0, 1);
        assert!(unsorted.encode().is_err());
        let mut oversized = maximum_credential();
        oversized.access_token.push('x');
        assert!(oversized.encode().is_err());
    }

    #[test]
    fn manifest_bounds_and_every_interrupted_publication_phase_recover_exactly() {
        assert_eq!(DESCRIPTOR_BYTES, 53);
        assert_eq!(MAX_MANIFEST_BYTES, 124);
        assert_eq!(MAX_MANIFEST_BYTES + 36, 160);
        let maximum = vec![b'x'; LOGICAL_CREDENTIAL_BYTES];
        let largest = descriptor(&maximum).unwrap();
        assert_eq!(largest.chunks, 27);
        let second = descriptor(b"second").unwrap();
        assert_eq!(
            encode_manifest(&Manifest::PreparedReplace {
                old: largest.clone(),
                new: second,
            })
            .len(),
            MAX_MANIFEST_BYTES
        );

        let items = SyntheticItems::default();
        let seed = [9; 32];
        let manifest = NativeSecretAccount::from_digest(seed);
        let old_bytes = vec![b'o'; MAX_NATIVE_SECRET_BYTES + 17];
        let first = vault(items.clone(), &manifest, seed)
            .create(old_bytes.clone())
            .unwrap();
        assert_eq!(items.count(), 3, "manifest plus two chunks");

        // A prepared replacement without complete new chunks rolls back and
        // retains the exact old readable generation.
        let incomplete_bytes = vec![b'i'; MAX_NATIVE_SECRET_BYTES + 1];
        let incomplete = descriptor(&incomplete_bytes).unwrap();
        {
            let mut vault = vault(items.clone(), &manifest, seed);
            let current = vault.recover().unwrap().unwrap();
            let Manifest::Stable(old) = current.state.clone() else {
                panic!("expected stable manifest")
            };
            vault
                .transition(
                    current,
                    Manifest::PreparedReplace {
                        old,
                        new: incomplete,
                    },
                )
                .unwrap();
        }
        let recovered = vault(items.clone(), &manifest, seed)
            .read()
            .unwrap()
            .unwrap();
        assert_eq!(recovered.generation(), first);
        assert_eq!(recovered.secret(), old_bytes);
        assert_eq!(items.count(), 3);

        // A prepared replacement whose chunks are complete commits new, then
        // removes every retired old chunk before reaching stable-new.
        let next_bytes = vec![b'n'; MAX_NATIVE_SECRET_BYTES * 2 + 1];
        let next = descriptor(&next_bytes).unwrap();
        {
            let mut vault = vault(items.clone(), &manifest, seed);
            let current = vault.recover().unwrap().unwrap();
            let Manifest::Stable(old) = current.state.clone() else {
                panic!("expected stable manifest")
            };
            vault
                .transition(
                    current,
                    Manifest::PreparedReplace {
                        old,
                        new: next.clone(),
                    },
                )
                .unwrap();
            vault.write_chunks(&next, &next_bytes).unwrap();
        }
        let recovered = vault(items.clone(), &manifest, seed)
            .read()
            .unwrap()
            .unwrap();
        assert_eq!(recovered.generation(), next.generation);
        assert_eq!(recovered.secret(), next_bytes);
        assert_eq!(items.count(), 4, "manifest plus three new chunks");

        // Once committed-new is visible, recovery can only finish retirement;
        // it never rolls back to the old generation.
        let final_bytes = vec![b'f'; MAX_NATIVE_SECRET_BYTES + 3];
        let final_descriptor = descriptor(&final_bytes).unwrap();
        {
            let mut vault = vault(items.clone(), &manifest, seed);
            let current = vault.recover().unwrap().unwrap();
            let Manifest::Stable(old) = current.state.clone() else {
                panic!("expected stable manifest")
            };
            let prepared = vault
                .transition(
                    current,
                    Manifest::PreparedReplace {
                        old: old.clone(),
                        new: final_descriptor.clone(),
                    },
                )
                .unwrap();
            vault.write_chunks(&final_descriptor, &final_bytes).unwrap();
            vault
                .transition(
                    prepared,
                    Manifest::CommittedNew {
                        new: final_descriptor.clone(),
                        retired: old,
                    },
                )
                .unwrap();
        }
        let recovered = vault(items.clone(), &manifest, seed)
            .read()
            .unwrap()
            .unwrap();
        assert_eq!(recovered.generation(), final_descriptor.generation);
        assert_eq!(recovered.secret(), final_bytes);
        assert_eq!(items.count(), 3, "manifest plus two final chunks");

        assert!(
            vault(items.clone(), &manifest, seed)
                .replace(first, b"stale-refresh".to_vec())
                .is_err()
        );
        assert_eq!(
            vault(items.clone(), &manifest, seed)
                .read()
                .unwrap()
                .unwrap()
                .secret(),
            final_bytes
        );

        // Committed deletion makes absence authoritative before chunk cleanup.
        {
            let mut vault = vault(items.clone(), &manifest, seed);
            let current = vault.recover().unwrap().unwrap();
            let Manifest::Stable(retired) = current.state.clone() else {
                panic!("expected stable manifest")
            };
            vault
                .transition(current, Manifest::CommittedDelete { retired })
                .unwrap();
        }
        assert!(
            vault(items.clone(), &manifest, seed)
                .read()
                .unwrap()
                .is_none()
        );
        assert_eq!(items.count(), 0, "manifest and retired chunks removed");
        assert!(!format!("{recovered:?}").contains("ffff"));
    }

    #[test]
    fn prepared_collision_preserves_foreign_record_and_does_not_strand_manifest() {
        let items = SyntheticItems::default();
        let seed = [17; 32];
        let manifest = NativeSecretAccount::from_digest(seed);
        let bytes = vec![b'p'; MAX_NATIVE_SECRET_BYTES + 1];
        let prepared = descriptor(&bytes).unwrap();
        let expected = native_secret_generation(prepared.generation).unwrap();
        let first_account = chunk_account(seed, prepared.generation, 0);
        let second_account = chunk_account(seed, prepared.generation, 1);
        let first =
            NativeSecretRecord::new(expected, bytes[..MAX_NATIVE_SECRET_BYTES].to_vec()).unwrap();
        let foreign_generation = NativeSecretGeneration::new([0x5a; 16]).unwrap();
        assert_ne!(foreign_generation, expected);
        let foreign = NativeSecretRecord::new(foreign_generation, b"foreign".to_vec()).unwrap();

        {
            let mut vault = vault(items.clone(), &manifest, seed);
            let native_generation = native_generation().unwrap();
            vault
                .store
                .create(
                    &manifest,
                    &manifest_record(
                        native_generation,
                        &Manifest::PreparedCreate {
                            new: prepared.clone(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            vault.store.create(&first_account, &first).unwrap();
            vault.store.create(&second_account, &foreign).unwrap();
            assert!(vault.write_chunks(&prepared, &bytes).is_err());
        }

        assert!(
            vault(items.clone(), &manifest, seed)
                .read()
                .unwrap()
                .is_none()
        );
        let records = items.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert!(
            records
                .iter()
                .any(|(account, record)| { account == &second_account && record == &foreign })
        );
        drop(records);

        let later = b"later credential".to_vec();
        let generation = vault(items.clone(), &manifest, seed)
            .create(later.clone())
            .unwrap();
        let loaded = vault(items.clone(), &manifest, seed)
            .read()
            .unwrap()
            .unwrap();
        assert_eq!(loaded.generation(), generation);
        assert_eq!(loaded.secret(), later);
        assert!(
            items
                .records
                .lock()
                .unwrap()
                .iter()
                .any(|(account, record)| { account == &second_account && record == &foreign })
        );
    }

    #[tokio::test]
    async fn lock_is_project_alias_bound_and_serializes_without_secret_bytes() {
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let private_parent =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let data = private_parent
            .create_private_directory(OsStr::new("data"))
            .unwrap();
        let root = Arc::new(
            Directory::open(project.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project.path(),
            None,
            None,
            None,
            &[],
            InvocationOverrides::default(),
        )
        .unwrap();
        let store = Arc::new(
            McpCredentialStore::new(data.path(), root, snapshot.manifest().full_digest()).unwrap(),
        );
        let held = store.acquire("first").await.unwrap();
        let other_alias = store.acquire("second").await.unwrap();
        drop(other_alias);
        let contender = tokio::spawn({
            let store = store.clone();
            async move { store.acquire("first").await }
        });
        tokio::time::sleep(Duration::from_millis(75)).await;
        assert!(!contender.is_finished());
        drop(held);
        let acquired = contender.await.unwrap().unwrap();
        drop(acquired);

        let lock_directory = data.path().join(&store.directory_name);
        let entries = std::fs::read_dir(lock_directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 2);
        for entry in entries {
            let bytes = std::fs::read(entry).unwrap();
            assert!(bytes.is_empty());
        }
    }
}
