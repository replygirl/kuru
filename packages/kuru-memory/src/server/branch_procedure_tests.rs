//! Textual backstop for the rule that a Dolt branch rename or delete is built
//! only from a [`SessionsEnded`] proof.
//!
//! The enforcement for typed routes is the compiler-checked token, not this
//! test: the compiler rejects constructing the proof outside `server.rs` (its
//! field is private), reusing one proof twice (rename and delete consume it)
//! and releasing the admission fence before the procedure has run (the
//! procedure borrows it). Making the raw pool close private is not itself a
//! guarantee: `Server::close_pool_without_session_end` is `pub(crate)` and does
//! exactly what the raw close does; it simply cannot produce a proof.
//!
//! The compiler cannot see SQL text, so this scan backstops raw SQL. In every
//! non-test source of the crate, outside `impl SessionsEnded` in `server.rs`,
//! it rejects any string literal, in either quote style, whose contents are a
//! rename, delete or force flag of `DOLT_BRANCH` (compared ASCII
//! case-insensitively and ignoring surrounding spaces). That covers the flag
//! written inline in SQL, bound as an argument, assigned to a variable or
//! constant, or placed on another line than the procedure name.
//!
//! The rule is file-wide rather than per function because function bounds
//! cannot be read reliably from text, and it applies to every product file,
//! not only files that name `dolt_branch`, so a flag constant defined in one
//! module and used in another is also rejected. Both choices are the stricter
//! option. The real product sources pass: branch creation passes names only.
//!
//! Limits: the scan sees only literal text. A flag assembled at run time (for
//! example `format!("-{}", "D")`, a concatenation, or bytes decoded from data)
//! passes it, as does a procedure name assembled at run time. Test files
//! (`*tests.rs`) are exempt because they construct fault states with raw
//! procedures.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Dolt branch procedure arguments that rename, delete or overwrite a branch.
/// Matched ASCII case-insensitively, so `-M` and `-F` are rejected as well.
const DESTRUCTIVE_FLAGS: &[&str] = &["-m", "--move", "-d", "-D", "--delete", "-f", "--force"];

/// Characters that open or close a string literal in Rust or SQL.
const QUOTES: &[char] = &['"', '\''];

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

/// Whether the flag at `line[start..end]` is the whole contents of a quoted
/// literal: after optional spaces, a quote on each side. Backslashes between
/// the flag and its closing quote are skipped, so an escaped `\"-D\"` inside
/// a Rust string counts; its opening backslash precedes the quote.
fn is_quoted(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].trim_end_matches([' ', '\t']);
    let after = line[end..].trim_start_matches([' ', '\t', '\\']);
    before.ends_with(QUOTES) && after.starts_with(QUOTES)
}

/// Every quoted destructive-flag literal in `text`, as `line:column: line`.
fn flag_literals(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        let mut columns: Vec<usize> = Vec::new();
        for flag in DESTRUCTIVE_FLAGS {
            let flag = flag.to_ascii_lowercase();
            for (start, _) in lower.match_indices(flag.as_str()) {
                if is_quoted(line, start, start + flag.len()) && !columns.contains(&start) {
                    columns.push(start);
                }
            }
        }
        columns.sort_unstable();
        for column in columns {
            found.push(format!("{}:{}: {}", index + 1, column + 1, line.trim()));
        }
    }
    found
}

/// Every violation in one file's text: a quoted destructive flag, or
/// (outside `server.rs`, where the compiler also rejects it) a struct literal
/// of the proof.
fn violations(text: &str, proof_literal_allowed: bool) -> Vec<String> {
    let mut found = flag_literals(text);
    if !proof_literal_allowed {
        for (index, line) in text.lines().enumerate() {
            if line.contains("SessionsEnded {") {
                found.push(format!("{}: {}", index + 1, line.trim()));
            }
        }
    }
    found
}

/// `server.rs` with the lines of `impl<'a> SessionsEnded<'a>`, the only place
/// a branch procedure may be written, blanked (line numbers are kept).
fn outside_proof_impl(text: &str) -> String {
    let mut inside = false;
    let mut found = false;
    let mut kept = String::new();
    for line in text.lines() {
        if line.starts_with("impl<'a> SessionsEnded<'a> {") {
            inside = true;
            found = true;
        } else if inside && line == "}" {
            inside = false;
        } else if !inside {
            kept.push_str(line);
        }
        kept.push('\n');
    }
    assert!(found, "server.rs has no `impl<'a> SessionsEnded<'a>` block");
    assert!(
        !inside,
        "the `impl SessionsEnded` block in server.rs never closed"
    );
    kept
}

#[test]
fn the_scan_rejects_every_raw_branch_rename_or_delete_shape() {
    for bad in [
        // Shapes reported by review of round 8 (each got 0 violations then).
        "        let flag = if force { \"-D\" } else { \"-d\" };\n        sqlx::query(\"CALL DOLT_BRANCH(?, ?)\")\n            .bind(flag)\n            .bind(branch)",
        "sqlx::query(\"CALL dolt_branch('-D', ?)\")",
        "sqlx::query(\"CALL DOLT_BRANCH(\\\"-D\\\", ?)\")",
        "sqlx::query(\"CALL DOLT_BRANCH(\n '-D', ?)\")",
        // Earlier shapes.
        "sqlx::query(\"CALL DOLT_BRANCH('-m', ?, ?)\")",
        "sqlx::query(\"CALL DOLT_BRANCH('--delete', ?)\")",
        ".bind(\"-d\")",
        "let proof = SessionsEnded { admission };",
        // A flag constant in a module that never names the procedure.
        "pub(crate) const FORCE_DELETE: &str = \"--delete\";",
        // A raw string literal with a long force flag.
        "sqlx::query(r#\"CALL DOLT_BRANCH('--force', ?, ?)\"#)",
        // Mixed-case procedure and upper-case move flag.
        "sqlx::query(\"CALL Dolt_Branch('-M', ?, ?)\")",
        // Upper-case long flag padded with spaces inside its quotes.
        "sqlx::query(\"CALL DOLT_BRANCH(' --MOVE ', ?, ?)\")",
        // A flag in an argument array bound in a loop.
        "for argument in [\"-f\", name, base] { query = query.bind(argument); }",
        // A flag returned by a helper, away from the procedure that binds it.
        "fn delete_flag() -> &'static str {\n    \"-D\"\n}",
    ] {
        assert!(!violations(bad, false).is_empty(), "scan missed {bad}");
    }
    for accepted in [
        "sqlx::query(\"CALL DOLT_BRANCH(?, ?)\").bind(&name).bind(&base)",
        "sqlx::query(\"CALL DOLT_BRANCH('candidate_test')\")",
        "sqlx::query(\"SELECT name FROM dolt_branches WHERE name = ?\")",
        "let name = \"--delete-me\"; let other = \"-dx\"; let third = \"x-d\";",
        "impl<'a> Proof<'a> { fn branch(&self) -> &'a str { self.name } }",
        "// a checked delete (`-d`) and a forced one (`-D`)",
    ] {
        assert_eq!(
            violations(accepted, false),
            Vec::<String>::new(),
            "scan rejected {accepted}"
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
            // The scan must see the real procedures: two `-m` renames and the
            // `-D`/`-d` delete flag, all inside `impl SessionsEnded`.
            assert_eq!(
                flag_literals(&text).len(),
                4,
                "server.rs should name exactly the rename, exclusion probe and \
                 two delete flags: {:?}",
                flag_literals(&text)
            );
            assert_eq!(
                text.matches("sqlx::query(\"CALL DOLT_BRANCH(").count(),
                3,
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
