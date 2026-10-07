//! The template identity marker, its reader and the template verdict type
//! (cospec change `memory-template-stage-adoption`). Engine-backed adoption
//! tests live in `store/template_stage_tests.rs`.
use super::*;
use sha2::{Digest, Sha256};

fn identity(template: Option<&str>) -> Identity {
    Identity {
        version: 1,
        instance: "4b0e1f0c-8f2a-4c55-9a51-3d3c3c0b7f10".into(),
        project_scope: format!("project/{}", "c".repeat(64)),
        password: "a".repeat(64),
        reader_password: "b".repeat(64),
        initialized: false,
        template: template.map(str::to_owned),
        source: None,
    }
}

/// The identity record as the type serialized it before the `template` field
/// existed, and as a binary that predates the field reads it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OlderIdentity {
    version: u32,
    instance: String,
    project_scope: String,
    password: String,
    reader_password: String,
    initialized: bool,
}

/// A store created directly or by import serializes its identity record
/// byte-for-byte as before the field existed, and reads an older record back
/// without a template; a template-born record carries the key, and a binary
/// that predates the field fails closed on it.
#[test]
fn cold_identity_record_bytes_are_unchanged() -> Result<()> {
    let cold = identity(None);
    let bytes = serde_json::to_vec(&cold)?;
    let older = serde_json::to_vec(&OlderIdentity {
        version: cold.version,
        instance: cold.instance.clone(),
        project_scope: cold.project_scope.clone(),
        password: cold.password.clone(),
        reader_password: cold.reader_password.clone(),
        initialized: cold.initialized,
    })?;
    assert_eq!(bytes, older, "a cold identity record changed its bytes");
    assert!(!String::from_utf8(bytes.clone())?.contains("template"));
    let read: Identity = serde_json::from_slice(&older)?;
    assert!(read.template.is_none());

    let marked = serde_json::to_vec(&identity(Some("key")))?;
    assert!(String::from_utf8(marked.clone())?.contains(r#""template":"key""#));
    assert!(
        serde_json::from_slice::<OlderIdentity>(&marked).is_err(),
        "a binary without the field must refuse a template-born identity"
    );
    assert!(serde_json::from_slice::<OlderIdentity>(&bytes).is_ok());
    Ok(())
}

#[test]
fn restored_identity_keeps_origin_separate_from_live_authority() -> Result<()> {
    let mut restored = identity(None);
    let origin_instance = Uuid::new_v4().to_string();
    let origin_scope = format!("project/{}", "d".repeat(64));
    restored.version = 2;
    restored.source = Some(SourceIdentity {
        instance: origin_instance.clone(),
        project_scope: origin_scope.clone(),
    });
    validate_identity(&restored)?;
    assert_eq!(
        restored.sql_identity(),
        (origin_instance.as_str(), origin_scope.as_str())
    );
    assert_ne!(restored.sql_identity().0, restored.instance);
    let bytes = serde_json::to_vec(&restored)?;
    assert!(serde_json::from_slice::<OlderIdentity>(&bytes).is_err());
    let roundtrip: Identity = serde_json::from_slice(&bytes)?;
    assert_eq!(roundtrip.sql_identity(), restored.sql_identity());

    let mut inconsistent = restored.clone();
    inconsistent.version = 1;
    assert!(validate_identity(&inconsistent).is_err());
    let mut reused = restored.clone();
    reused.instance = origin_instance;
    assert!(validate_identity(&reused).is_err());
    let mut template = restored;
    template.template = Some("key".into());
    assert!(validate_identity(&template).is_err());
    Ok(())
}

/// `stage_template_key` reads the key through the one identity type and its
/// checked reader: `None` without a record or a key, the key when present,
/// and an error for an unreadable, unknown or invalid record. The writers
/// publish the copy and build identities and refuse to replace a record.
#[test]
fn stage_template_key_reads_the_identity_record() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let directory = root.path().join("stage");
    private_directory(&directory)?;
    let record = directory.join("identity.json");
    assert_eq!(stage_template_key(&directory)?, None);

    write_record(&record, &identity(None))?;
    assert_eq!(stage_template_key(&directory)?, None);
    write_record(&record, &identity(Some("key-1")))?;
    assert_eq!(stage_template_key(&directory)?.as_deref(), Some("key-1"));
    // A SHA-256 hex key at the longest bound is one valid path component.
    for valid in [
        "0123456789abcdef".repeat(4),
        "k_".repeat(TEMPLATE_KEY_LIMIT / 2),
    ] {
        write_record(&record, &identity(Some(&valid)))?;
        assert_eq!(stage_template_key(&directory)?, Some(valid));
    }

    for invalid in [
        b"{not json".to_vec(),
        br#"{"version":1,"unexpected":true}"#.to_vec(),
        serde_json::to_vec(&identity(Some("")))?,
        serde_json::to_vec(&identity(Some("line\nbreak")))?,
        serde_json::to_vec(&identity(Some(&"k".repeat(TEMPLATE_KEY_LIMIT + 1))))?,
        serde_json::to_vec(&identity(Some("Key")))?,
        serde_json::to_vec(&identity(Some("a/b")))?,
        serde_json::to_vec(&identity(Some("a\\b")))?,
        serde_json::to_vec(&identity(Some("..")))?,
        serde_json::to_vec(&identity(Some("key.lock")))?,
        serde_json::to_vec(&identity(Some("with space")))?,
        serde_json::to_vec(&identity(Some("kéy")))?,
        serde_json::to_vec(&Identity {
            instance: "not-a-uuid".into(),
            ..identity(Some("key"))
        })?,
    ] {
        files::write(&record, &invalid)?;
        assert!(
            stage_template_key(&directory).is_err(),
            "accepted {:?}",
            String::from_utf8_lossy(&invalid)
        );
    }

    let stage = root.path().join("copy");
    private_directory(&stage)?;
    let scope = format!("project/{}", "d".repeat(64));
    write_template_stage_identity(&stage, &scope, "key-2")?;
    let written = load_identity(&stage, &scope)?.context("no copy identity")?;
    assert!(!written.initialized);
    assert_eq!(written.template.as_deref(), Some("key-2"));
    assert_eq!(Uuid::parse_str(&written.instance)?.get_version_num(), 4);
    assert!(write_template_stage_identity(&stage, &scope, "key-2").is_err());

    let build = root.path().join("build");
    private_directory(&build)?;
    write_template_build_identity(&build, "key-3")?;
    let written = load_identity(&build, TEMPLATE_SCOPE)?.context("no build identity")?;
    assert_eq!(written.instance, TEMPLATE_INSTANCE);
    assert_eq!(written.template.as_deref(), Some("key-3"));
    assert!(!written.initialized);
    assert!(write_template_build_identity(&build, "key-3").is_err());
    Ok(())
}

/// The placeholder is the nil instance and the digest scope of its tag, a
/// valid project scope that names no fixture project; the compiled key is a
/// valid template key.
#[test]
fn template_placeholder_is_the_compiled_literal() -> Result<()> {
    assert!(Uuid::parse_str(TEMPLATE_INSTANCE)?.is_nil());
    let digest = Sha256::digest(b"kuru-memory-store-template-scope-v1")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(TEMPLATE_SCOPE, format!("project/{digest}"));
    crate::store::project_directory(Path::new("data"), TEMPLATE_SCOPE)?;
    assert_ne!(TEMPLATE_SCOPE, crate::store::temporary_scope());
    validate_identity(&identity(Some(compiled_template_key())))?;
    Ok(())
}

/// A verdict is found through any context, on either side of the process
/// boundary; any other error is not a verdict.
#[test]
fn template_verdict_is_found_through_contexts() {
    let error = anyhow::Error::from(TemplateVerdict::new("placeholder differs"))
        .context("Dolt database bootstrap failed")
        .context("Dolt startup/lifetime failed");
    assert_eq!(
        TemplateVerdict::find(&error).map(ToString::to_string),
        Some("placeholder differs".to_owned())
    );
    assert!(format!("{error:#}").ends_with("placeholder differs"));
    let other = anyhow!("memory SQL project/instance identity mismatch").context("open");
    assert!(TemplateVerdict::find(&other).is_none());
}
