## Context

Background, measurements and the proven recipe are in the PR6 research note (cgo is
required; static ICU 78.3 with stub data keeps `dolt.exe` about 118 MB under the
128 MiB expanded bound; llvm-mingw target runtimes embed host-package paths, so one
build host is pinned). Current constraints:

- Two independent `deny_unknown_fields` parsers read `dolt-assets.json`:
  `packages/kuru-memory/support/bundle.rs` (build.rs, catalog, tests; exactly 5 assets,
  fixed target→stem map, dolthub URL rule, zip `exe + license == expanded`) and
  `packages/kuru-delivery/src/bundle.rs` (1–32 assets, HTTPS URL rule).
- `provision.rs::extract_zip` requires exactly four members; `kuru-archive` ZIP rules
  (Unix creator, no descriptors, no ZIP64, no comments) are satisfied by
  `kuru_archive::zip::write`.
- The built archive's pins can only come from the linux-x64 CI job, so the change lands
  in two CI rounds.

## Goals / Non-Goals

**Goals:** a built provenance that cannot perturb the five upstream targets; a
fail-closed representation of "not yet pinned"; a reproducible build whose pins are
established and enforced only by linux-x64 CI.

**Non-Goals:** selecting `aarch64-pc-windows-msvc` as a Rust or release target (PR6b);
hosting or publishing built archives; a Dolt version bump; PGO parity with upstream.

## Decisions

1. **Explicit `provenance` field, upstream entries otherwise unchanged.** Each asset
   gains `"provenance": "upstream" | "built"` directly after `executable_name`; all
   existing upstream keys, values and order (including flat `url`) stay. Missing or
   unknown provenance is rejected. Rejected: nesting `url` under `source: {kind, url}`
   (rewrites upstream entries); defaulting absent to upstream (not fail-closed).
2. **Built entries.** `url` is forbidden; `build` is required:
   `recipe` (allowlist `dolt-cgo-llvm-mingw-icu-stub/1`), `host` (`linux-x64`),
   `goos`, `goarch`, `tags` (`["icu_static","timetzdata"]`),
   `sources.dolt.{module,version,sum}` (module `github.com/dolthub/dolt/go`; version a
   pseudo-version ending in `upstream_commit[..12]`; sum `h1:` + 44 base64),
   `sources.icu.{version,url,bytes,sha256}`,
   `toolchain.go.{version,url,sha256}`,
   `toolchain.llvm_mingw.{version,clang_version,url,bytes,sha256}`.
   The command checks `go version` and `aarch64-w64-mingw32-clang --version` output;
   tarball digests exist so a test can assert equality with
   `packages/kuru-memory/mise.lock`. Rejected: relying on mise.lock alone (the manifest
   would not self-describe provenance and the two could drift silently).
3. **Two-round pinning with an explicit sentinel.** On built assets only,
   `archive_sha256`, `executable_sha256` and notice `sha256` may be `"unpinned"` with
   the matching sizes `null`. Preparation and build.rs refuse an unpinned asset with the
   actionable build-task instruction; `bundle build --print-pins` reports observed pins;
   the CI verify step prints a visible "unpinned" notice instead of skipping silently.
   Upstream assets can never be unpinned. Rejected: zero digests or the macOS probe hash
   (look like real pins; the Linux build differs).
4. **Notices beside `LICENSES`.** `notices: [{name, from, path, bytes, sha256}]`,
   required and non-empty for built, forbidden for upstream. `name` is the member
   basename under `{stem}/` (`LICENSE-ICU`, `LICENSE-LLVM`, `LICENSE-MINGW-W64-RUNTIME`);
   `from` is `icu` (path inside the extracted pinned ICU tarball) or `llvm_mingw` (path
   under the mise-installed toolchain root). Built zip size rule:
   `exe + license + Σ notices == expanded`; upstream rule unchanged. `LICENSES` stays
   upstream `Godeps/LICENSES`, and the built asset's `license_sha256` must equal every
   upstream asset's. Runtime `Asset` gains a notices list (empty for upstream) and
   `extract_zip` accepts exactly the four legacy members plus declared notices.
   Rejected: appending notices into `LICENSES` (breaks the upstream parity digest);
   a docs-only notice (does not travel with the binary).
5. **Build in `kuru-delivery`, orchestrated by `kuru-memory`.** `bundle build` is a new
   `BundleCommand::Build` in the independent helper; it never compiles or links
   `kuru-memory`. It clears inherited toolchain environment (`CC`, `CXX`, `*FLAGS`,
   `CGO_*`, `GOFLAGS`, `GOPRIVATE`, `GONOSUMDB`, `GOINSECURE`, `PKG_CONFIG_PATH`), sets
   `GOTOOLCHAIN=local GOENV=off GOWORK=off GOPROXY=https://proxy.golang.org
   GOSUMDB=sum.golang.org` with private caches, fetches the module by
   `go mod download -json` and compares `Sum`, asserts `doltversion.Version == version`,
   streams ICU through the existing bounded HTTPS client, builds host ICU then cross ICU
   with `-ffile-prefix-map`, links `libsicuin.a`, `libsicuuc.a` and `stubdata/libsicudt.a`,
   runs `CGO_ENABLED=1 GOOS=windows GOARCH=arm64 go build -trimpath -buildvcs=false
   -tags icu_static,timetzdata -ldflags="-s -w -buildid="` with
   `CGO_CXXFLAGS="-O2 -g -include cstdlib"` and `CGO_LDFLAGS=-static`, checks PE machine
   `0xAA64` and the import allowlist, and packages with `kuru_archive::zip::write`. The
   work directory is private under the bundle directory or `RUNNER_TEMP`, never under
   runtime memory settings. The mise task `bundle:build` scopes Go and llvm-mingw
   (linux-x64 asset pattern only) to itself; `setup:build-tools` mirrors the delivery
   package's `--include-task-tools` precedent. Rejected: a shell or Docker recipe
   (outside the Rust helper's verified environment and tests).
6. **Separate workflow file.** `.github/workflows/bundle-build.yml` runs one
   `ubuntu-latest` job that builds twice into two fresh private directories (separate
   work trees, Go caches and module caches), then requires identical archive
   bytes, prints SHA-256 and size, uploads the archive, and verifies the committed pin
   when present. Actions are pinned by SHA; `KURU_MBX=0`. Rejected: editing `ci.yml`
   (concurrently edited by other changes; PR6b wires consumers).

## Operational surface

- Runner: GitHub-hosted `ubuntu-latest` (x86_64) only, in the new
  `bundle-build.yml`; no containers, no self-hosted runners, no bind addresses or
  listening services. `bundle build` refuses any host other than linux-x64.
- Secrets: none. Only the default read-only `GITHUB_TOKEN` for checkout and artifact
  upload; workflow `permissions: contents: read`.
- Tool versions and arches: Go 1.26.2 linux-amd64, llvm-mingw 20260922 UCRT
  ubuntu-22.04-x86_64 (clang 23.1.2), mise and Rust from the repository pins; target
  is windows/arm64 PE (`0xAA64`). Tool installation is task-scoped
  (`setup:build-tools`) and locked by `packages/kuru-memory/mise.lock`.
- Network: the module proxy `proxy.golang.org`, checksum database `sum.golang.org`,
  and the ICU GitHub release asset, each over HTTPS with the existing bounded client
  or the go command; downloads are size- and digest-checked. Offline mode refuses to
  build. Runtime provisioning makes no network calls.
- Limits: archive ≤ 64 MiB compressed, ≤ 128 MiB expanded; job timeouts bounded in
  the workflow (measured on round 1); private work directories under `RUNNER_TEMP`,
  never restored from caches.

## Integration contract

- Go module proxy: `go mod download -json github.com/dolthub/dolt/go@<version>` must
  report `Sum` equal to the manifest `h1:`; `GOSUMDB=sum.golang.org` additionally
  verifies it; the module's `go.sum` under `-mod=readonly` pins transitive modules.
- ICU: immutable release asset
  `release-78.3/icu4c-78.3-sources.tgz`, pinned by byte size and SHA-256; notice path
  `icu/LICENSE` inside it.
- llvm-mingw: tarball identity owned by `packages/kuru-memory/mise.lock` and mirrored
  in the manifest `toolchain.llvm_mingw` (a test asserts equality); notice paths are
  relative to the installed toolchain root.
- Archive contract with runtime: members `{stem}/`, `{stem}/bin/`,
  `{stem}/bin/dolt.exe`, `{stem}/LICENSES`, then each declared notice, written by
  `kuru_archive::zip::write` and accepted by `kuru-archive`'s decoder and
  `provision.rs::extract_zip`. Upstream archive contracts are unchanged.
- Schema reconciliation: both manifest parsers accept the same v2 schema and reject v1;
  field names are fixed in Decisions 1–4. Fixtures: fake Go/clang executables, a local
  HTTP fixture for the proxy and ICU, and generated built-layout ZIPs.

## Risks / Trade-offs

- [Linux-to-Linux nondeterminism across runner images] → two independent builds must
  agree before pinning; later drift fails closed against the pin.
- [`archive_sha256` coupled to `zip`/`flate2` pins] → documented in the lockfile
  refresh procedure; executable and notice digests are unaffected and a stale archive
  pin fails the build loudly.
- [llvm-mingw Linux package notice paths or clang version differ from the macOS probe]
  → round 1 prints them; they stay `unpinned` until confirmed.
- [winpthreads statically linked without a notice] → round 1 prints link inputs; add
  `COPYING.winpthreads.txt` as a fourth notice before pinning if it is present.
- [Size headroom about 15 MB under 128 MiB] → bound checks unchanged; growth fails
  loudly rather than raising caps.
- [New Go and C toolchains in the repository] → confined to one package task and one
  build host, as permitted for necessary tooling.

## Open Questions

- Whether PR builds should rebuild the engine on every run or restore the pinned archive
  from cache and re-verify (Q4); deferrable, as it changes only workflow triggers.
