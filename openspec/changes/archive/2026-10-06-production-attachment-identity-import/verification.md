# Verification

## 1. Default production compilation [critical]

- [x] 1.1 @regression (agent) run the owning default-feature production release build after the import correction -> PR243 baseline native build/install logs fail with E0425/E0433 at service.rs437/444; corrected owning app release build handle 34766 exited 0, cargo build -p kuru --release --locked, without test-support or all-features, KURU_MBX=0. Release compiler completed in 8m24s; owning task 570.16s. Two existing dead-code warnings for fixture_commit_malformed_state at facade.rs3094/store.rs4818 were emitted; no other compile errors.
- [x] 1.2 @integration (agent) run affected static checks and inspect the exact import diff -> host lint 79894, Windows lint 13191, formatting 49675, docs 13605 and managed 3907 exited 0; source diff removes only the cfg attribute above the existing Arc import.

## 2. Native CI follow-through

- [~] 2.1 @runtime (agent) verify the fresh fixed head in native build and installation CI -> defer: native target builds/installations execute remotely after clean branch handoff; local host compilation is not native deployment evidence.
