## 1. Manifest schema v2 parsing and provenance branching [critical]

- [ ] 1.1 @unit (agent) both parsers accept the committed v2 manifest and reject v1, missing/unknown `provenance`, `url` on built, `build` or `notices` on upstream, unknown recipe, non-linux-x64 host, pseudo-version not ending in `upstream_commit[..12]`, malformed `h1:` sum, bad ICU digest, built `license_sha256` differing from upstream, and unknown fields -> each case yields its specific error
- [ ] 1.2 @unit (agent) `"unpinned"`/`null` sentinels accepted only on built assets and rejected on upstream; mixed pinned/unpinned archive fields rejected -> errors observed
- [ ] 1.3 @unit (agent) bound checks: built compressed > 64 MiB, expanded > 128 MiB, and `exe + license + Σ notices != expanded` rejected; upstream zip rule unchanged -> errors observed
- [ ] 1.4 @integration (agent) `bundle prepare` for each upstream target against a manifest whose built entry is unpinned runs with a live HTTP fixture and never requests or reads built inputs; `bundle prepare --target aarch64-pc-windows-msvc` refuses with the build-task instruction -> fixture request log and message observed
- [ ] 1.5 @regression (agent) the five upstream entries differ from main only by the inserted `"provenance": "upstream",` line (`git diff` on `dolt-assets.json`) and `catalog()` values for those targets are unchanged -> diff and test observed

## 2. Third-party notices required for built assets [critical]

- [ ] 2.1 @unit (agent) built asset with empty or missing `notices`, duplicate or invalid notice `name`, `from` outside `{icu, llvm_mingw}`, or escaping `path` rejected -> errors observed
- [ ] 2.2 @integration (agent) runtime `extract_zip` on a fixture built-layout archive accepts exactly the declared notices with exact sizes, and rejects a missing, extra or resized notice; the real upstream windows-amd64 ZIP fixture still extracts unchanged -> tests observed
- [ ] 2.3 @unit (agent) manifest toolchain pins (`go`, `llvm_mingw` url/sha256) equal `packages/kuru-memory/mise.lock` entries -> test observed

## 3. Source build command [critical]

- [ ] 3.1 @integration (agent) `bundle build` with fake Go/clang toolchains and a fake module proxy/ICU server: refuses non-linux-x64 host and upstream assets, refuses offline mode, rejects wrong `Sum`, wrong ICU digest, wrong `go version`/clang version, non-ARM64 PE and disallowed imports, clears inherited toolchain environment, and writes the exact member layout -> subprocess fixtures observed
- [ ] 3.2 @runtime (agent) GitHub Actions `ubuntu-latest` run of `mise run //packages/kuru-memory:bundle:build -- --target aarch64-pc-windows-msvc --print-pins` with the real pinned toolchains builds `dolt.exe` (PE 0xAA64) and prints pins including clang version and link inputs -> run ID and printed pins recorded

## 4. Reproducible pins in CI [critical]

- [ ] 4.1 @runtime (agent) `bundle-build.yml` builds twice on `ubuntu-latest` in fresh private directories and `cmp` proves byte-identical archives; the job prints SHA-256 and size and uploads the archive -> run ID, digest and size recorded
- [ ] 4.2 @runtime (agent) after the pins are committed (task P), the same workflow verifies the built archive, executable and notices against the manifest pins and passes -> run ID recorded
- [ ] 4.3 @integration (agent) the pin-verify step fails and reports both digests when fed an archive differing from the pin, and prints a visible unpinned notice when the manifest is unpinned -> local step invocation observed

## 5. Existing targets unaffected [critical]

- [ ] 5.1 @runtime (agent) every existing CI job for the five upstream targets (ci.yml static categories, native tests and coverage shards, installation) passes on the PR head -> run IDs recorded
- [ ] 5.2 @regression (agent) root `mise.lock` unchanged and `mise run cospec:managed:check`, `lint:tooling` pass locally -> commands observed

## 6. Documentation

- [ ] 6.1 @unit (agent) `mise run docs:check` passes with the updated bundled build inputs section naming linux-x64 as the only build host and the zip/flate2 re-pin rule -> command observed
