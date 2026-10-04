#![recursion_limit = "256"]

mod authentication;
pub mod cli;
mod commands;
mod diagnostics;
mod instruction_gate;
pub mod memory_activity;
mod memory_export;
mod memory_notice;
mod permission_store;
mod session_export;
#[cfg(test)]
mod spawn_gate;
mod trust;
pub mod ui;

#[cfg(test)]
mod tests {
    /// Every async test in the library, and in each integration test that
    /// opens a memory store in this process, runs its body in
    /// `kuru_memory::test_support::closing`. A store dropped live hands its
    /// Dolt supervisor to a detached reaper that can outlive the test process
    /// and write a late coverage profile; the scope closes every store the
    /// body opened on every exit path and names a test that left one unawaited.
    #[test]
    fn every_store_opening_async_test_runs_its_body_in_the_closing_scope() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sources = kuru_memory::test_support::rust_sources(&root.join("src"));
        sources.extend(
            kuru_memory::test_support::rust_sources(&root.join("tests"))
                .into_iter()
                .filter(|path| {
                    std::fs::read_to_string(path)
                        .unwrap()
                        .contains("MemoryStore::")
                }),
        );
        kuru_memory::test_support::assert_async_tests_run_in_closing(root, &sources);
    }
}
