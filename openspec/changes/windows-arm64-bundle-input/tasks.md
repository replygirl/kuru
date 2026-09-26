## 1. Manifest schema v2

- [ ] 1.1 (T1) Implement schema v2 with explicit `provenance`, the `build` object, `"unpinned"` sentinels and the validators in design.md in both `packages/kuru-memory/support/bundle.rs` and `packages/kuru-delivery/src/bundle.rs`; add the inert `aarch64-pc-windows-msvc` built entry and raise the kuru-memory asset count to 6; branch preparation on provenance so other targets never read built inputs and unpinned built assets are refused; verify with verification rows 1.1–1.5
- [ ] 1.2 (T2) Add notices handling: parser rules, built zip size rule, runtime `Asset` notices list (empty for upstream) and notice-aware `provision.rs::extract_zip`; verify with verification rows 2.1–2.2

## 2. Source build

- [ ] 2.1 (T3) Add `kuru-delivery bundle build` (host gate, cleared child environment, module fetch with `Sum` and version-constant checks, bounded ICU fetch, host and cross ICU builds, cgo `go build`, PE machine and import checks, `kuru_archive::zip::write` packaging, pin verification, `--print-pins`, private work directory); verify with verification row 3.1
- [ ] 2.2 (T4) Add task-scoped Go 1.26.2 and llvm-mingw 20260922 UCRT (linux-x64 asset only) plus `setup:build-tools` and `bundle:build` to `packages/kuru-memory/mise.toml`, and create `packages/kuru-memory/mise.lock` for linux-x64 without touching the root `mise.lock`; verify with verification rows 2.3 and 5.2

## 3. CI and documentation

- [ ] 3.1 (T5) Add `.github/workflows/bundle-build.yml` with one `ubuntu-latest` job that builds twice into fresh private directories, compares, uploads and verifies the pin (SHA-pinned actions, `KURU_MBX=0`); verify with actionlint via `mise run lint:tooling` and verification rows 3.2, 4.1 and 4.3
- [ ] 3.2 (T6) Update the `docs/development.md` bundled engine build inputs section (built provenance, `bundle:build`, linux-x64 only build host, offline import of a built archive, zip/flate2 re-pin in the lockfile refresh procedure); verify with verification row 6.1

## 4. Pinning

- [ ] 4.1 (P) Commit the pins printed by the CI round-1 build job (archive, executable and llvm-mingw notice sizes/digests; confirm clang version, notice paths and whether winpthreads needs a notice) and confirm the round-2 run verifies them; verify with verification rows 4.2 and 5.1
