//! Exact Kuru-owned records in the host's native credential facility.
//!
//! The account is an opaque domain-separated digest supplied by the connector.
//! This module never enumerates credentials and never accepts another service
//! namespace. Callers serialize cross-process mutations before using the
//! generation checks provided here.

use std::{error::Error, fmt};

const SERVICE: &str = "io.kuru.mcp.oauth";
const MAGIC: &[u8; 16] = b"KURU-MCP-SECRET\0";
const ACCOUNT_HEX_BYTES: usize = 64;
// Windows Credential Manager is the narrowest supported native backend. Its
// CRED_MAX_CREDENTIAL_BLOB_SIZE is 5 * 512 bytes. The complete encoded item,
// including this module's generation and length envelope, must fit that bound.
const MAX_NATIVE_ITEM_BYTES: usize = 5 * 512;
const ENVELOPE_BYTES: usize = MAGIC.len() + 16 + 4;
pub const MAX_NATIVE_SECRET_BYTES: usize = MAX_NATIVE_ITEM_BYTES - ENVELOPE_BYTES;

#[derive(Clone, Eq, PartialEq)]
pub struct NativeSecretAccount(String);

impl NativeSecretAccount {
    pub fn new(value: String) -> Result<Self, NativeSecretError> {
        if value.len() != ACCOUNT_HEX_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(NativeSecretError::new(NativeSecretErrorKind::Corrupt));
        }
        Ok(Self(value))
    }

    pub fn from_digest(digest: [u8; 32]) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut value = String::with_capacity(ACCOUNT_HEX_BYTES);
        for byte in digest {
            value.push(char::from(HEX[usize::from(byte >> 4)]));
            value.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        Self(value)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NativeSecretAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeSecretAccount([opaque])")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeSecretGeneration([u8; 16]);

impl NativeSecretGeneration {
    pub fn new(bytes: [u8; 16]) -> Result<Self, NativeSecretError> {
        if bytes == [0; 16] {
            return Err(NativeSecretError::new(NativeSecretErrorKind::Corrupt));
        }
        Ok(Self(bytes))
    }

    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Debug for NativeSecretGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeSecretGeneration([opaque])")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct NativeSecretRecord {
    generation: NativeSecretGeneration,
    secret: Vec<u8>,
}

impl NativeSecretRecord {
    pub fn new(
        generation: NativeSecretGeneration,
        secret: Vec<u8>,
    ) -> Result<Self, NativeSecretError> {
        if secret.is_empty() || secret.len() > MAX_NATIVE_SECRET_BYTES {
            return Err(NativeSecretError::new(NativeSecretErrorKind::Corrupt));
        }
        Ok(Self { generation, secret })
    }

    pub const fn generation(&self) -> NativeSecretGeneration {
        self.generation
    }

    pub fn secret(&self) -> &[u8] {
        &self.secret
    }
}

impl fmt::Debug for NativeSecretRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSecretRecord")
            .field("generation", &self.generation)
            .field("secret", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSecretErrorKind {
    NotFound,
    Stale,
    Denied,
    Unavailable,
    Corrupt,
}

#[derive(Debug)]
pub struct NativeSecretError {
    kind: NativeSecretErrorKind,
}

impl NativeSecretError {
    const fn new(kind: NativeSecretErrorKind) -> Self {
        Self { kind }
    }

    pub const fn kind(&self) -> NativeSecretErrorKind {
        self.kind
    }
}

impl fmt::Display for NativeSecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeSecretErrorKind::NotFound => "native secret record was not found",
            NativeSecretErrorKind::Stale => "native secret record generation changed",
            NativeSecretErrorKind::Denied => "native secret store denied the operation",
            NativeSecretErrorKind::Unavailable => "native secret store is unavailable",
            NativeSecretErrorKind::Corrupt => "native secret record is invalid",
        })
    }
}

impl Error for NativeSecretError {}

pub struct NativeSecretStore {
    store: Store<PlatformBackend>,
}

impl NativeSecretStore {
    pub const fn new() -> Self {
        Self {
            store: Store::new(PlatformBackend),
        }
    }

    pub fn get(
        &self,
        account: &NativeSecretAccount,
    ) -> Result<NativeSecretRecord, NativeSecretError> {
        self.store.get(account)
    }

    pub fn create(
        &self,
        account: &NativeSecretAccount,
        record: &NativeSecretRecord,
    ) -> Result<(), NativeSecretError> {
        self.store.create(account, record)
    }

    pub fn replace(
        &self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
        record: &NativeSecretRecord,
    ) -> Result<(), NativeSecretError> {
        self.store.replace(account, expected, record)
    }

    pub fn delete(
        &self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
    ) -> Result<(), NativeSecretError> {
        self.store.delete(account, expected)
    }
}

impl Default for NativeSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

struct Store<B> {
    backend: B,
}

impl<B> Store<B> {
    const fn new(backend: B) -> Self {
        Self { backend }
    }
}

impl<B: Backend> Store<B> {
    fn get(&self, account: &NativeSecretAccount) -> Result<NativeSecretRecord, NativeSecretError> {
        let bytes = self.backend.get(account)?;
        decode(&bytes)
    }

    fn create(
        &self,
        account: &NativeSecretAccount,
        record: &NativeSecretRecord,
    ) -> Result<(), NativeSecretError> {
        match self.backend.get(account) {
            Err(error) if error.kind() == NativeSecretErrorKind::NotFound => {}
            Ok(_) => return Err(NativeSecretError::new(NativeSecretErrorKind::Stale)),
            Err(error) => return Err(error),
        }
        self.write_and_verify(account, record)
    }

    fn replace(
        &self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
        record: &NativeSecretRecord,
    ) -> Result<(), NativeSecretError> {
        let current = self.get(account)?;
        if current.generation != expected || record.generation == expected {
            return Err(NativeSecretError::new(NativeSecretErrorKind::Stale));
        }
        self.write_and_verify(account, record)
    }

    fn delete(
        &self,
        account: &NativeSecretAccount,
        expected: NativeSecretGeneration,
    ) -> Result<(), NativeSecretError> {
        let current = self.get(account)?;
        if current.generation != expected {
            return Err(NativeSecretError::new(NativeSecretErrorKind::Stale));
        }
        self.backend.delete(account)?;
        match self.backend.get(account) {
            Err(error) if error.kind() == NativeSecretErrorKind::NotFound => Ok(()),
            Ok(_) => Err(NativeSecretError::new(NativeSecretErrorKind::Stale)),
            Err(error) => Err(error),
        }
    }

    fn write_and_verify(
        &self,
        account: &NativeSecretAccount,
        record: &NativeSecretRecord,
    ) -> Result<(), NativeSecretError> {
        let encoded = encode(record)?;
        self.backend.set(account, &encoded)?;
        if self.backend.get(account)? != encoded {
            return Err(NativeSecretError::new(NativeSecretErrorKind::Corrupt));
        }
        Ok(())
    }
}

trait Backend {
    fn get(&self, account: &NativeSecretAccount) -> Result<Vec<u8>, NativeSecretError>;
    fn set(&self, account: &NativeSecretAccount, bytes: &[u8]) -> Result<(), NativeSecretError>;
    fn delete(&self, account: &NativeSecretAccount) -> Result<(), NativeSecretError>;
}

struct PlatformBackend;

impl PlatformBackend {
    fn entry(account: &NativeSecretAccount) -> Result<keyring::Entry, NativeSecretError> {
        keyring::Entry::new(SERVICE, account.as_str()).map_err(map_keyring_error)
    }
}

impl Backend for PlatformBackend {
    fn get(&self, account: &NativeSecretAccount) -> Result<Vec<u8>, NativeSecretError> {
        Self::entry(account)?
            .get_secret()
            .map_err(map_keyring_error)
    }

    fn set(&self, account: &NativeSecretAccount, bytes: &[u8]) -> Result<(), NativeSecretError> {
        Self::entry(account)?
            .set_secret(bytes)
            .map_err(map_keyring_error)
    }

    fn delete(&self, account: &NativeSecretAccount) -> Result<(), NativeSecretError> {
        Self::entry(account)?
            .delete_credential()
            .map_err(map_keyring_error)
    }
}

fn map_keyring_error(error: keyring::Error) -> NativeSecretError {
    use keyring::Error;

    let kind = match error {
        Error::NoEntry => NativeSecretErrorKind::NotFound,
        Error::NoStorageAccess(_) => NativeSecretErrorKind::Denied,
        Error::NoDefaultStore | Error::NotSupportedByStore(_) | Error::PlatformFailure(_) => {
            NativeSecretErrorKind::Unavailable
        }
        Error::BadEncoding(_)
        | Error::BadDataFormat(_, _)
        | Error::BadStoreFormat(_)
        | Error::Ambiguous(_) => NativeSecretErrorKind::Corrupt,
        Error::Invalid(_, _) | Error::TooLong(_, _) => NativeSecretErrorKind::Denied,
        _ => NativeSecretErrorKind::Unavailable,
    };
    NativeSecretError::new(kind)
}

fn encode(record: &NativeSecretRecord) -> Result<Vec<u8>, NativeSecretError> {
    if record.secret.is_empty() || record.secret.len() > MAX_NATIVE_SECRET_BYTES {
        return Err(NativeSecretError::new(NativeSecretErrorKind::Corrupt));
    }
    let length = u32::try_from(record.secret.len())
        .map_err(|_| NativeSecretError::new(NativeSecretErrorKind::Corrupt))?;
    let mut bytes = Vec::with_capacity(MAGIC.len() + 16 + 4 + record.secret.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&record.generation.as_bytes());
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(&record.secret);
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<NativeSecretRecord, NativeSecretError> {
    if bytes.len() < ENVELOPE_BYTES || &bytes[..MAGIC.len()] != MAGIC {
        return Err(NativeSecretError::new(NativeSecretErrorKind::Corrupt));
    }
    let generation = NativeSecretGeneration::new(
        bytes[16..32]
            .try_into()
            .map_err(|_| NativeSecretError::new(NativeSecretErrorKind::Corrupt))?,
    )?;
    let length = usize::try_from(u32::from_be_bytes(
        bytes[32..36]
            .try_into()
            .map_err(|_| NativeSecretError::new(NativeSecretErrorKind::Corrupt))?,
    ))
    .map_err(|_| NativeSecretError::new(NativeSecretErrorKind::Corrupt))?;
    if length == 0 || length > MAX_NATIVE_SECRET_BYTES || bytes.len() != ENVELOPE_BYTES + length {
        return Err(NativeSecretError::new(NativeSecretErrorKind::Corrupt));
    }
    NativeSecretRecord::new(generation, bytes[ENVELOPE_BYTES..].to_vec())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        hash::{Hash, Hasher},
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[derive(Clone, Default)]
    struct SyntheticBackend {
        records: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    }

    impl Backend for SyntheticBackend {
        fn get(&self, account: &NativeSecretAccount) -> Result<Vec<u8>, NativeSecretError> {
            self.records
                .lock()
                .unwrap()
                .get(account.as_str())
                .cloned()
                .ok_or_else(|| NativeSecretError::new(NativeSecretErrorKind::NotFound))
        }

        fn set(
            &self,
            account: &NativeSecretAccount,
            bytes: &[u8],
        ) -> Result<(), NativeSecretError> {
            self.records
                .lock()
                .unwrap()
                .insert(account.as_str().to_owned(), bytes.to_vec());
            Ok(())
        }

        fn delete(&self, account: &NativeSecretAccount) -> Result<(), NativeSecretError> {
            if self
                .records
                .lock()
                .unwrap()
                .remove(account.as_str())
                .is_some()
            {
                Ok(())
            } else {
                Err(NativeSecretError::new(NativeSecretErrorKind::NotFound))
            }
        }
    }

    fn generation(byte: u8) -> NativeSecretGeneration {
        NativeSecretGeneration::new([byte; 16]).unwrap()
    }

    fn record(generation: u8, value: &[u8]) -> NativeSecretRecord {
        NativeSecretRecord::new(self::generation(generation), value.to_vec()).unwrap()
    }

    fn unique_account(domain: u8) -> NativeSecretAccount {
        let seed = (
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            domain,
        );
        let mut digest = [0; 32];
        for chunk in 0..4_u8 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            seed.hash(&mut hasher);
            chunk.hash(&mut hasher);
            digest[usize::from(chunk) * 8..usize::from(chunk + 1) * 8]
                .copy_from_slice(&hasher.finish().to_be_bytes());
        }
        NativeSecretAccount::from_digest(digest)
    }

    struct NativeCleanup {
        accounts: Vec<NativeSecretAccount>,
        foreign_service: String,
        foreign_account: NativeSecretAccount,
    }

    impl Drop for NativeCleanup {
        fn drop(&mut self) {
            let store = NativeSecretStore::new();
            for account in &self.accounts {
                if let Ok(record) = store.get(account) {
                    let _ = store.delete(account, record.generation());
                }
            }
            if let Ok(foreign) =
                keyring::Entry::new(&self.foreign_service, self.foreign_account.as_str())
            {
                let _ = foreign.delete_credential();
            }
        }
    }

    #[test]
    fn exact_record_create_replace_delete_is_generation_checked_and_redacted() {
        let backend = SyntheticBackend::default();
        let store = Store::new(backend.clone());
        let account = NativeSecretAccount::from_digest([7; 32]);
        assert_eq!(
            store.get(&account).unwrap_err().kind(),
            NativeSecretErrorKind::NotFound
        );

        let first = record(1, b"recognizable-secret-one");
        store.create(&account, &first).unwrap();
        assert_eq!(store.get(&account).unwrap(), first);
        assert_eq!(
            store
                .create(&account, &record(2, b"loser"))
                .unwrap_err()
                .kind(),
            NativeSecretErrorKind::Stale
        );
        assert_eq!(
            store
                .replace(&account, generation(2), &record(3, b"loser"))
                .unwrap_err()
                .kind(),
            NativeSecretErrorKind::Stale
        );

        let second = record(2, b"recognizable-secret-two");
        store.replace(&account, generation(1), &second).unwrap();
        assert_eq!(store.get(&account).unwrap(), second);
        assert_eq!(
            store.delete(&account, generation(1)).unwrap_err().kind(),
            NativeSecretErrorKind::Stale
        );
        store.delete(&account, generation(2)).unwrap();
        assert_eq!(
            store.get(&account).unwrap_err().kind(),
            NativeSecretErrorKind::NotFound
        );

        let debug = format!("{first:?} {account:?} {:?}", generation(1));
        assert!(!debug.contains("recognizable-secret"));
        assert!(!debug.contains(&account.0));
    }

    #[test]
    fn record_codec_rejects_invalid_bounds_generation_and_shape() {
        assert!(NativeSecretAccount::new("not-a-digest".into()).is_err());
        assert!(NativeSecretGeneration::new([0; 16]).is_err());
        assert!(NativeSecretRecord::new(generation(1), Vec::new()).is_err());
        assert!(
            NativeSecretRecord::new(generation(1), vec![0; MAX_NATIVE_SECRET_BYTES + 1]).is_err()
        );
        assert_eq!(
            encode(&record(1, &vec![0; MAX_NATIVE_SECRET_BYTES]))
                .unwrap()
                .len(),
            MAX_NATIVE_ITEM_BYTES
        );
        assert_eq!(
            decode(b"malformed").unwrap_err().kind(),
            NativeSecretErrorKind::Corrupt
        );

        let mut encoded = encode(&record(1, b"value")).unwrap();
        encoded.push(0);
        assert_eq!(
            decode(&encoded).unwrap_err().kind(),
            NativeSecretErrorKind::Corrupt
        );
    }

    #[derive(Default)]
    struct FaultBackend {
        inner: SyntheticBackend,
        reads: Mutex<std::collections::VecDeque<Result<Vec<u8>, NativeSecretErrorKind>>>,
        write_error: Option<NativeSecretErrorKind>,
        delete_error: Option<NativeSecretErrorKind>,
        retain_deleted: bool,
    }

    impl Backend for FaultBackend {
        fn get(&self, account: &NativeSecretAccount) -> Result<Vec<u8>, NativeSecretError> {
            match self.reads.lock().unwrap().pop_front() {
                Some(value) => value.map_err(NativeSecretError::new),
                None => self.inner.get(account),
            }
        }

        fn set(
            &self,
            account: &NativeSecretAccount,
            bytes: &[u8],
        ) -> Result<(), NativeSecretError> {
            if let Some(kind) = self.write_error {
                return Err(NativeSecretError::new(kind));
            }
            self.inner.set(account, bytes)
        }

        fn delete(&self, account: &NativeSecretAccount) -> Result<(), NativeSecretError> {
            if let Some(kind) = self.delete_error {
                return Err(NativeSecretError::new(kind));
            }
            if self.retain_deleted {
                Ok(())
            } else {
                self.inner.delete(account)
            }
        }
    }

    #[test]
    fn uncertain_secret_backend_outcomes_never_claim_verified_write_or_deletion() {
        let account = NativeSecretAccount::from_digest([1; 32]);
        let adjacent = NativeSecretAccount::from_digest([2; 32]);
        let original = record(1, b"original-fake-secret");
        let replacement = record(2, b"replacement-fake-secret");
        let sentinel = record(3, b"adjacent-fake-secret");
        for kind in [
            NativeSecretErrorKind::Denied,
            NativeSecretErrorKind::Unavailable,
            NativeSecretErrorKind::Corrupt,
        ] {
            let backend = FaultBackend::default();
            backend
                .inner
                .set(&adjacent, &encode(&sentinel).unwrap())
                .unwrap();
            backend.reads.lock().unwrap().push_back(Err(kind));
            let store = Store::new(backend);
            let error = store.create(&account, &original).unwrap_err();
            assert_eq!(error.kind(), kind);
            assert!(!error.to_string().contains("fake-secret"));
            assert_eq!(store.get(&adjacent).unwrap(), sentinel);
            assert_eq!(
                store.get(&account).unwrap_err().kind(),
                NativeSecretErrorKind::NotFound
            );
        }
        let backend = FaultBackend {
            write_error: Some(NativeSecretErrorKind::Denied),
            ..Default::default()
        };
        backend
            .inner
            .set(&account, &encode(&original).unwrap())
            .unwrap();
        let store = Store::new(backend);
        assert_eq!(
            store
                .replace(&account, generation(1), &replacement)
                .unwrap_err()
                .kind(),
            NativeSecretErrorKind::Denied
        );
        assert_eq!(store.get(&account).unwrap(), original);
        // A native write returning success can still have an uncertain result.
        // Its readback must match the complete generation and secret envelope.
        let backend = FaultBackend::default();
        backend.reads.lock().unwrap().extend([
            Err(NativeSecretErrorKind::NotFound),
            Ok(encode(&replacement).unwrap()),
        ]);
        let store = Store::new(backend);
        assert_eq!(
            store.create(&account, &original).unwrap_err().kind(),
            NativeSecretErrorKind::Corrupt
        );
        assert_eq!(store.get(&account).unwrap(), original);
        for (retained, failure) in [
            (true, None),
            (false, Some(NativeSecretErrorKind::Unavailable)),
        ] {
            let backend = FaultBackend {
                retain_deleted: retained,
                ..Default::default()
            };
            backend
                .inner
                .set(&account, &encode(&original).unwrap())
                .unwrap();
            backend
                .inner
                .set(&adjacent, &encode(&sentinel).unwrap())
                .unwrap();
            if let Some(kind) = failure {
                backend
                    .reads
                    .lock()
                    .unwrap()
                    .extend([Ok(encode(&original).unwrap()), Err(kind)]);
            }
            let store = Store::new(backend);
            assert_eq!(
                store.delete(&account, generation(1)).unwrap_err().kind(),
                failure.unwrap_or(NativeSecretErrorKind::Stale)
            );
            assert_eq!(store.get(&adjacent).unwrap(), sentinel);
            if retained {
                assert_eq!(store.get(&account).unwrap(), original);
            }
        }
        let backend = FaultBackend {
            delete_error: Some(NativeSecretErrorKind::Denied),
            ..Default::default()
        };
        backend
            .inner
            .set(&account, &encode(&original).unwrap())
            .unwrap();
        let store = Store::new(backend);
        assert_eq!(
            store.delete(&account, generation(1)).unwrap_err().kind(),
            NativeSecretErrorKind::Denied
        );
        assert_eq!(store.get(&account).unwrap(), original);
    }

    #[test]
    fn native_secret_errors_are_typed_and_do_not_disclose_backend_payloads() {
        use keyring::Error;
        let cases = [
            (Error::NoEntry, NativeSecretErrorKind::NotFound),
            (
                Error::NoStorageAccess(Box::new(std::io::Error::other("fake-sensitive-data"))),
                NativeSecretErrorKind::Denied,
            ),
            (Error::NoDefaultStore, NativeSecretErrorKind::Unavailable),
            (
                Error::NotSupportedByStore("fake-sensitive-data".into()),
                NativeSecretErrorKind::Unavailable,
            ),
            (
                Error::BadEncoding(b"fake-sensitive-data".to_vec()),
                NativeSecretErrorKind::Corrupt,
            ),
            (
                Error::BadDataFormat(
                    b"fake-sensitive-data".to_vec(),
                    Box::new(std::io::Error::other("private account")),
                ),
                NativeSecretErrorKind::Corrupt,
            ),
            (
                Error::BadStoreFormat("fake-sensitive-data".into()),
                NativeSecretErrorKind::Corrupt,
            ),
            (Error::Ambiguous(Vec::new()), NativeSecretErrorKind::Corrupt),
            (
                Error::PlatformFailure(Box::new(std::io::Error::other("fake-sensitive-data"))),
                NativeSecretErrorKind::Unavailable,
            ),
            (
                Error::Invalid("fake-sensitive-data".into(), "private account".into()),
                NativeSecretErrorKind::Denied,
            ),
            (
                Error::TooLong("fake-sensitive-data".into(), 1),
                NativeSecretErrorKind::Denied,
            ),
        ];
        for (backend, kind) in cases {
            let error = map_keyring_error(backend);
            assert_eq!(error.kind(), kind);
            assert!(!format!("{error:?} {error}").contains("fake-sensitive-data"));
        }
    }

    #[test]
    fn native_store_reopens_replaces_deletes_and_isolates_adjacent_records() {
        let account = unique_account(1);
        let adjacent = unique_account(2);
        let foreign_service = format!("io.kuru.test.foreign.{}", std::process::id());
        let _cleanup = NativeCleanup {
            accounts: vec![account.clone(), adjacent.clone()],
            foreign_service: foreign_service.clone(),
            foreign_account: account.clone(),
        };
        let foreign = keyring::Entry::new(&foreign_service, account.as_str()).unwrap();
        foreign.set_secret(b"foreign-recognizable-secret").unwrap();

        let store = NativeSecretStore::new();
        assert_eq!(
            store.get(&account).unwrap_err().kind(),
            NativeSecretErrorKind::NotFound
        );
        let first = record(3, b"native-recognizable-secret-one");
        let other = record(4, b"native-recognizable-secret-adjacent");
        store.create(&account, &first).unwrap();
        store.create(&adjacent, &other).unwrap();
        assert_eq!(NativeSecretStore::default().get(&account).unwrap(), first);
        assert_eq!(
            store
                .create(&account, &record(5, b"stale"))
                .unwrap_err()
                .kind(),
            NativeSecretErrorKind::Stale
        );

        let second = record(5, b"native-recognizable-secret-two");
        store.replace(&account, generation(3), &second).unwrap();
        assert_eq!(store.get(&account).unwrap(), second);
        assert_eq!(store.get(&adjacent).unwrap(), other);
        assert_eq!(
            foreign.get_secret().unwrap(),
            b"foreign-recognizable-secret"
        );

        store.delete(&account, generation(5)).unwrap();
        store.delete(&adjacent, generation(4)).unwrap();
        foreign.delete_credential().unwrap();
        assert_eq!(
            store.get(&account).unwrap_err().kind(),
            NativeSecretErrorKind::NotFound
        );
    }

    mod record_admission_contracts {
        use super::*;

        #[test]
        fn structured_diagnostics_and_error_sources_never_expose_secret_material() {
            let account = NativeSecretAccount::from_digest([11; 32]);
            let adjacent = NativeSecretAccount::from_digest([12; 32]);
            let payload = b"fake-diagnostic-secret\n{\"token\":\"private\"}";
            let original = record(13, payload);
            let sentinel = record(14, b"fake-adjacent-diagnostic-record");
            let backend = FaultBackend::default();
            backend
                .inner
                .set(&account, &encode(&original).unwrap())
                .unwrap();
            backend
                .inner
                .set(&adjacent, &encode(&sentinel).unwrap())
                .unwrap();
            let store = Store::new(backend);
            let diagnostic = format!("{:#?}", store.get(&account).unwrap());
            assert_eq!(store.get(&account).unwrap().secret(), payload);
            assert!(diagnostic.contains("[redacted]"));
            assert!(diagnostic.contains("[opaque]"));
            assert!(!diagnostic.contains("fake-diagnostic-secret"));
            assert!(!diagnostic.contains(account.as_str()));
            for kind in [
                NativeSecretErrorKind::NotFound,
                NativeSecretErrorKind::Stale,
                NativeSecretErrorKind::Denied,
                NativeSecretErrorKind::Unavailable,
                NativeSecretErrorKind::Corrupt,
            ] {
                store.backend.reads.lock().unwrap().push_back(Err(kind));
                let error = store.get(&account).unwrap_err();
                assert_eq!(error.kind(), kind);
                assert!(std::error::Error::source(&error).is_none());
                let diagnostic = format!("{error} {error:#?}");
                assert!(!diagnostic.contains("fake-diagnostic-secret"));
                assert!(!diagnostic.contains(account.as_str()));
                assert_eq!(store.get(&account).unwrap(), original);
                assert_eq!(store.get(&adjacent).unwrap(), sentinel);
            }
            assert_eq!(store.backend.inner.records.lock().unwrap().len(), 2);
        }

        #[test]
        fn literal_accounts_admit_only_exact_lowercase_digest_bytes() {
            let backend = SyntheticBackend::default();
            let store = Store::new(backend.clone());
            let literal = "a".repeat(ACCOUNT_HEX_BYTES);
            let admitted = NativeSecretAccount::new(literal.clone()).unwrap();
            assert_eq!(admitted.as_str(), literal);
            let original = record(1, b"fake-account-sentinel");
            store.create(&admitted, &original).unwrap();
            for invalid in [
                "A".repeat(ACCOUNT_HEX_BYTES),
                "g".repeat(ACCOUNT_HEX_BYTES),
                "\0".repeat(ACCOUNT_HEX_BYTES),
                "é".repeat(ACCOUNT_HEX_BYTES / 2),
            ] {
                assert_eq!(invalid.len(), ACCOUNT_HEX_BYTES);
                let error = NativeSecretAccount::new(invalid).unwrap_err();
                assert_eq!(error.kind(), NativeSecretErrorKind::Corrupt);
                assert_eq!(store.get(&admitted).unwrap(), original);
                assert_eq!(backend.records.lock().unwrap().len(), 1);
            }
            store.delete(&admitted, original.generation()).unwrap();
            assert!(backend.records.lock().unwrap().is_empty());
        }

        #[test]
        fn corrupt_native_envelopes_remain_intact_across_every_mutating_admission() {
            let account = NativeSecretAccount::from_digest([1; 32]);
            let adjacent = NativeSecretAccount::from_digest([2; 32]);
            let original = record(3, b"recognizable-fake-envelope-payload");
            let replacement = record(4, b"fake-replacement");
            let sentinel = record(5, b"fake-adjacent-record");
            let encoded = encode(&original).unwrap();
            for fault in 0..7 {
                let mut corrupt = encoded.clone();
                match fault {
                    0 => corrupt[0] ^= 0x80,
                    1 => corrupt[MAGIC.len()..MAGIC.len() + 16].fill(0),
                    2 => corrupt[MAGIC.len() + 16..ENVELOPE_BYTES].fill(0),
                    3 => corrupt[MAGIC.len() + 16..ENVELOPE_BYTES]
                        .copy_from_slice(&((MAX_NATIVE_SECRET_BYTES + 1) as u32).to_be_bytes()),
                    4 => {
                        corrupt.pop();
                    }
                    5 => corrupt.push(0),
                    _ => corrupt.truncate(ENVELOPE_BYTES - 1),
                }
                let backend = SyntheticBackend::default();
                backend.set(&account, &corrupt).unwrap();
                let store = Store::new(backend.clone());
                store.create(&adjacent, &sentinel).unwrap();
                for error in [
                    store.get(&account).unwrap_err(),
                    store
                        .replace(&account, original.generation(), &replacement)
                        .unwrap_err(),
                    store.delete(&account, original.generation()).unwrap_err(),
                ] {
                    assert_eq!(
                        error.kind(),
                        NativeSecretErrorKind::Corrupt,
                        "fault {fault}"
                    );
                    let diagnostic = format!("{error} {error:?}");
                    assert!(!diagnostic.contains("recognizable-fake-envelope-payload"));
                    assert!(!diagnostic.contains(account.as_str()));
                }
                assert_eq!(
                    store.create(&account, &replacement).unwrap_err().kind(),
                    NativeSecretErrorKind::Stale,
                );
                assert_eq!(backend.get(&account).unwrap(), corrupt);
                assert_eq!(store.get(&adjacent).unwrap(), sentinel);
                assert_eq!(backend.records.lock().unwrap().len(), 2);
            }
        }

        #[test]
        fn accepted_writes_with_failed_readback_preserve_uncertain_effects() {
            let account = NativeSecretAccount::from_digest([1; 32]);
            let adjacent = NativeSecretAccount::from_digest([2; 32]);
            let original = record(1, b"fake-accepted-create");
            let replacement = record(2, b"fake-accepted-replacement");
            let sentinel = record(3, b"fake-adjacent-sentinel");
            for kind in [
                NativeSecretErrorKind::Denied,
                NativeSecretErrorKind::Unavailable,
            ] {
                let backend = FaultBackend::default();
                backend
                    .inner
                    .set(&adjacent, &encode(&sentinel).unwrap())
                    .unwrap();
                backend
                    .reads
                    .lock()
                    .unwrap()
                    .extend([Err(NativeSecretErrorKind::NotFound), Err(kind)]);
                let store = Store::new(backend);
                assert_eq!(store.create(&account, &original).unwrap_err().kind(), kind);
                assert_eq!(store.get(&account).unwrap(), original);
                store
                    .backend
                    .reads
                    .lock()
                    .unwrap()
                    .extend([Ok(encode(&original).unwrap()), Err(kind)]);
                assert_eq!(
                    store
                        .replace(&account, original.generation(), &replacement)
                        .unwrap_err()
                        .kind(),
                    kind,
                );
                assert_eq!(store.get(&account).unwrap(), replacement);
                assert_eq!(store.get(&adjacent).unwrap(), sentinel);
                assert_eq!(store.backend.inner.records.lock().unwrap().len(), 2);
            }
        }
    }
}
