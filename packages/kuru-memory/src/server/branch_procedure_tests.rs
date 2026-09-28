//! Structural check that a Dolt branch rename or delete is built only from a
//! [`SessionsEnded`] proof.
//!
//! The compiler already rejects constructing the proof outside `server.rs`
//! (its field is private), calling the raw pool retirement (private), reusing
//! one proof twice (rename and delete consume it) and releasing the admission
//! fence before the procedure has run (the procedure borrows it). It cannot
//! see a raw SQL string, so this test rejects any other source of a branch
//! rename or delete in the crate's non-test code. Test files (`*tests.rs`)
//! may still issue raw procedures to construct fault states.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Dolt branch procedure arguments that rename, delete or overwrite a branch.
const DESTRUCTIVE_FLAGS: &[&str] = &["-m", "--move", "-d", "-D", "--delete", "-f", "--force"];

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read source directory") {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// Every violation in one file's text: a `DOLT_BRANCH` call naming a
/// destructive flag, a destructive flag bound as an argument, or (outside
/// `server.rs`, where the compiler also rejects it) a struct literal of the
/// proof.
fn violations(text: &str, proof_literal_allowed: bool) -> Vec<String> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        for flag in DESTRUCTIVE_FLAGS {
            let quoted = format!("'{flag}'");
            let bound = format!(".bind(\"{flag}\")");
            if (line.contains("DOLT_BRANCH") && line.contains(&quoted)) || line.contains(&bound) {
                found.push(format!("{line_number}: {}", line.trim()));
            }
        }
        if !proof_literal_allowed && line.contains("SessionsEnded {") {
            found.push(format!("{line_number}: {}", line.trim()));
        }
    }
    found
}

/// The lines of `server.rs` inside `impl<'a> SessionsEnded<'a>`, the only
/// place a branch procedure may be written.
fn outside_proof_impl(text: &str) -> String {
    let mut inside = false;
    let mut kept = String::new();
    for line in text.lines() {
        if line.starts_with("impl<'a> SessionsEnded<'a> {") {
            inside = true;
        } else if inside && line == "}" {
            inside = false;
        } else if !inside {
            kept.push_str(line);
        }
        kept.push('\n');
    }
    kept
}

#[test]
fn the_scan_rejects_every_raw_branch_rename_or_delete_shape() {
    for bad in [
        "sqlx::query(\"CALL DOLT_BRANCH('-m', ?, ?)\")",
        "sqlx::query(\"CALL DOLT_BRANCH('-D', ?)\")",
        "sqlx::query(\"CALL DOLT_BRANCH('--delete', ?)\")",
        ".bind(\"-d\")",
        "let proof = SessionsEnded { admission };",
    ] {
        assert_eq!(violations(bad, false).len(), 1, "scan missed {bad}");
    }
    for create in [
        "sqlx::query(\"CALL DOLT_BRANCH(?, ?)\").bind(&name).bind(&base)",
        "sqlx::query(\"CALL DOLT_BRANCH('candidate_test')\")",
    ] {
        assert!(
            violations(create, false).is_empty(),
            "scan rejected {create}"
        );
    }
}

#[test]
fn branch_renames_and_deletes_are_built_only_from_a_sessions_ended_proof() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    let server = root.join("server.rs");
    assert!(files.contains(&server), "server.rs was not scanned");
    let mut rejected = Vec::new();
    let mut scanned = 0;
    for file in &files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.ends_with("tests.rs") {
            continue;
        }
        scanned += 1;
        let text = fs::read_to_string(file).expect("read source file");
        let is_server = *file == server;
        let text = if is_server {
            let procedures = text.matches("sqlx::query(\"CALL DOLT_BRANCH(").count();
            assert_eq!(
                procedures, 3,
                "server.rs should build exactly the rename, exclusion probe and delete"
            );
            outside_proof_impl(&text)
        } else {
            text
        };
        for violation in violations(&text, is_server) {
            rejected.push(format!(
                "{}:{violation}",
                file.strip_prefix(&root).unwrap_or(file).display()
            ));
        }
    }
    assert!(scanned > 20, "too few product sources scanned: {scanned}");
    assert!(
        rejected.is_empty(),
        "a Dolt branch rename or delete must be built from SessionsEnded \
         (Server::retire_branch_sessions), never written directly:\n{}",
        rejected.join("\n")
    );
}
