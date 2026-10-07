use super::*;

fn scope() -> String {
    format!("project/{}", "a".repeat(64))
}

#[test]
fn project_structure_inspection_is_bounded_and_does_not_create_state() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let data = sandbox.path().join("absent-data");
    let scope = scope();

    assert_eq!(
        MemoryStore::inspect_project_structure(&data, &scope),
        ProjectStructure::Absent
    );
    assert!(!data.exists(), "inspection created the data directory");

    let root = files::ensure_private_directory(&data)?;
    drop(root);
    let memory = files::ensure_private_directory(&data.join("memory"))?;
    drop(memory);
    let project = project_directory(&data, &scope)?;
    let directory = files::ensure_private_directory(&project)?;
    drop(directory);
    assert_eq!(
        MemoryStore::inspect_project_structure(&data, &scope),
        ProjectStructure::InvalidActivation
    );
    assert!(!data.join("memory/locks").exists());

    files::write(
        &project.join("ready.json"),
        &serde_json::to_vec(&Activation {
            format: 1,
            project_scope: scope.clone(),
            initial_revision: "0".repeat(32),
            migration: None,
            history_scope: None,
            restore: None,
        })?,
    )?;
    assert_eq!(
        MemoryStore::inspect_project_structure(&data, &scope),
        ProjectStructure::Activated
    );

    files::write(
        &project.join("ready.json"),
        serde_json::json!({"format": 1, "project_scope": "project/not-this-project"})
            .to_string()
            .as_bytes(),
    )?;
    assert_eq!(
        MemoryStore::inspect_project_structure(&data, &scope),
        ProjectStructure::InvalidActivation
    );
    assert!(!data.join("tools").exists());
    assert!(!data.join("memory/services").exists());
    Ok(())
}
