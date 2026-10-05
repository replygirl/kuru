# Dependency audit

Cargo and mise-declared tool releases were checked on 2026-10-05 against the official crates.io sparse registry, npm registry, upstream GitHub releases and mise release metadata. Direct dependencies are exactly pinned; lockfiles resolve their compatible transitive dependencies. Rust dependencies use stable releases. The docs app follows cospec's VitePress 2 preview architecture and pins the latest available alpha explicitly.

The Dolt change checked SQLx 0.9.0, UUID 1.26.1 and full Dolt 2.3.3 on
2026-09-10. The memory package's catalog records the measured native archive,
executable and license digests for each cataloged target. The Windows change
also pins the native API bindings, strict ZIP codec and ConPTY test driver below.
The bundled engine moved to full Dolt 2.3.4 (release commit `373301a5`) on
2026-09-27 for its `journal_writer` bootstrap fix on cancelled contexts
([dolthub/dolt#11796](https://github.com/dolthub/dolt/pull/11796)) and its
manifest-root preservation during automatic garbage collection
([dolthub/dolt#11747](https://github.com/dolthub/dolt/issues/11747)). The four
upstream archives were re-pinned from the v2.3.4 release assets; their
`LICENSES` payload is byte-identical to 2.3.3. The Windows arm64 entry builds
from Go module `v0.40.5-0.20260914203517-373301a56c9a`, and its archive pins
are committed from the reproducible linux-x64 build.
It moved to full Dolt 2.3.5 (release commit `ad65af6c`) on 2026-09-27
because 2.3.5 is the current upstream release (published 2026-09-16); its
release notes contain nothing aimed at the storage path, and it is not
claimed to fix the macOS uncertain-write failure, which already recurred on
2.3.4. The four upstream archives were re-pinned from the v2.3.5 release
assets; their `LICENSES` payload is byte-identical to 2.3.4. The Windows
arm64 entry builds from Go module `v0.40.5-0.20260916160725-ad65af6cc937`,
and its archive pins come from the reproducible linux-x64 build.
The offline tokenizer addition checked `tiktoken-rs` 0.12.0 on 2026-09-22;
its embedded `o200k_base.tiktoken` SHA-256 matches [OpenAI's published
asset hash](https://github.com/openai/tiktoken/blob/main/tiktoken_ext/openai_public.py),
`446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`.
The 0.12.1 update, checked on 2026-09-26, embeds the same asset bytes and MIT
license. The connector package carries the crate and asset MIT attributions in
`THIRD_PARTY_NOTICES.md`.

## Rust dependencies

| Dependency           | Latest stable pin  | Registry                                                         |
| -------------------- | ------------------ | ---------------------------------------------------------------- |
| `anyhow`             | `1.0.104`          | [sparse index](https://index.crates.io/an/yh/anyhow)             |
| `async-trait`        | `0.1.92`           | [sparse index](https://index.crates.io/as/yn/async-trait)        |
| `axum`               | `0.8.9`            | [sparse index](https://index.crates.io/ax/um/axum)               |
| `base64`             | `0.23.1`           | [sparse index](https://index.crates.io/ba/se/base64)             |
| `cap-fs-ext`         | `4.0.3`            | [sparse index](https://index.crates.io/ca/p-/cap-fs-ext)         |
| `cap-std`            | `4.0.3`            | [sparse index](https://index.crates.io/ca/p-/cap-std)            |
| `clap`               | `4.6.7`            | [sparse index](https://index.crates.io/cl/ap/clap)               |
| `crc32fast`          | `1.5.2`            | [sparse index](https://index.crates.io/cr/c3/crc32fast)          |
| `crossterm`          | `0.29.0`           | [sparse index](https://index.crates.io/cr/os/crossterm)          |
| `flate2`             | `1.1.10`           | [sparse index](https://index.crates.io/fl/at/flate2)             |
| `futures`            | `0.3.34`           | [sparse index](https://index.crates.io/fu/tu/futures)            |
| `getrandom`          | `0.4.3`            | [sparse index](https://index.crates.io/ge/tr/getrandom)          |
| `grep-matcher`       | `0.1.9`            | [sparse index](https://index.crates.io/gr/ep/grep-matcher)       |
| `grep-regex`         | `0.1.14`           | [sparse index](https://index.crates.io/gr/ep/grep-regex)         |
| `grep-searcher`      | `0.1.17`           | [sparse index](https://index.crates.io/gr/ep/grep-searcher)      |
| `httpdate`           | `1.0.3`            | [sparse index](https://index.crates.io/ht/tp/httpdate)           |
| `ignore`             | `0.4.33`           | [sparse index](https://index.crates.io/ig/no/ignore)             |
| `jsonschema`         | `0.58.5`           | [sparse index](https://index.crates.io/js/on/jsonschema)         |
| `keyring`            | `4.2.0`            | [sparse index](https://index.crates.io/ke/yr/keyring)            |
| `nix`                | `0.31.3`           | [sparse index](https://index.crates.io/3/n/nix)                  |
| `portable-pty`       | `0.9.0`            | [sparse index](https://index.crates.io/po/rt/portable-pty)       |
| `ratatui`            | `0.30.2`           | [sparse index](https://index.crates.io/ra/ta/ratatui)            |
| `rcgen`              | `0.14.10`          | [sparse index](https://index.crates.io/rc/ge/rcgen)              |
| `reqwest`            | `0.13.5`           | [sparse index](https://index.crates.io/re/qw/reqwest)            |
| `roxmltree`          | `0.21.1`           | [sparse index](https://index.crates.io/ro/xm/roxmltree)          |
| `rusqlite`           | `0.40.2`           | [sparse index](https://index.crates.io/ru/sq/rusqlite)           |
| `rustix`             | `1.1.5`            | [sparse index](https://index.crates.io/ru/st/rustix)             |
| `scraper`            | `0.27.0`           | [sparse index](https://index.crates.io/sc/ra/scraper)            |
| `serde`              | `1.0.229`          | [sparse index](https://index.crates.io/se/rd/serde)              |
| `serde_json`         | `1.0.151`          | [sparse index](https://index.crates.io/se/rd/serde_json)         |
| `serde_yaml_ng`      | `0.10.0`           | [sparse index](https://index.crates.io/se/rd/serde_yaml_ng)      |
| `sha2`               | `0.11.0`           | [sparse index](https://index.crates.io/sh/a2/sha2)               |
| `sqlx`               | `0.9.0`            | [sparse index](https://index.crates.io/sq/lx/sqlx)               |
| `sqlx-core`          | `0.9.0`            | [sparse index](https://index.crates.io/sq/lx/sqlx-core)          |
| `sysinfo`            | `0.39.6`           | [sparse index](https://index.crates.io/sy/si/sysinfo)            |
| `tar`                | `0.4.46`           | [sparse index](https://index.crates.io/3/t/tar)                  |
| `tempfile`           | `3.27.0`           | [sparse index](https://index.crates.io/te/mp/tempfile)           |
| `tiktoken-rs`        | `0.12.1`           | [sparse index](https://index.crates.io/ti/kt/tiktoken-rs)        |
| `tokio`              | `1.53.2`           | [sparse index](https://index.crates.io/to/ki/tokio)              |
| `tokio-rustls`       | `0.26.6`           | [sparse index](https://index.crates.io/to/ki/tokio-rustls)       |
| `toml`               | `1.1.6+spec-1.1.0` | [sparse index](https://index.crates.io/to/ml/toml)               |
| `tower`              | `0.5.3`            | [sparse index](https://index.crates.io/to/we/tower)              |
| `tracing`            | `0.1.44`           | [sparse index](https://index.crates.io/tr/ac/tracing)            |
| `tracing-subscriber` | `0.3.23`           | [sparse index](https://index.crates.io/tr/ac/tracing-subscriber) |
| `unicode-width`      | `0.2.2`            | [sparse index](https://index.crates.io/un/ic/unicode-width)      |
| `url`                | `2.5.8`            | [sparse index](https://index.crates.io/3/u/url)                  |
| `usage-argv`         | `6.12.0`           | [sparse index](https://index.crates.io/us/ag/usage-argv)         |
| `usage-cli`          | `6.12.0`           | [sparse index](https://index.crates.io/us/ag/usage-cli)          |
| `usage-lib`          | `6.12.0`           | [sparse index](https://index.crates.io/us/ag/usage-lib)          |
| `uuid`               | `1.27.0`           | [sparse index](https://index.crates.io/uu/id/uuid)               |
| `vt100`              | `0.16.2`           | [sparse index](https://index.crates.io/vt/10/vt100)              |
| `windows-sys`        | `0.61.2`           | [sparse index](https://index.crates.io/wi/nd/windows-sys)        |
| `zip`                | `8.6.0`            | [sparse index](https://index.crates.io/3/z/zip)                  |

## Development tools

| Tool                                       | Stable pin       |
| ------------------------------------------ | ---------------- |
| `rust`                                     | `1.99.0`         |
| `github:aligned-team/cospec`               | `0.8.3`          |
| `aqua:jdx/hk`                              | `2.5.0`          |
| `aqua:tamasfe/taplo`                       | `0.10.0`         |
| `aqua:koalaman/shellcheck`                 | `0.11.0`         |
| `aqua:rhysd/actionlint`                    | `1.7.12`         |
| `aqua:taiki-e/cargo-llvm-cov`              | `0.9.1`          |
| `mr-boxington`                             | `1.22.0`         |
| `cargo:cargo-audit` (delivery package)     | `0.22.2`         |
| Node (docs app only)                       | `26.10.0`        |
| npm (docs app only)                        | `12.2.0`         |
| VitePress (docs app)                       | `2.0.0-alpha.20` |
| `vitepress-plugin-llms` (docs app)         | `1.14.0`         |
| `oxfmt` (docs app)                         | `0.70.0`         |
| `oxlint` (docs app)                        | `1.85.0`         |
| Cocogitto (delivery package)               | `7.0.0`          |
| Communiqué (delivery package)              | `1.5.0`          |
| Go (memory source-build task only)         | `1.27.1`         |
| llvm-mingw (memory source-build task only) | `20260922`       |
| mise (CI; root `min_version` soft)         | `2026.9.18`      |

mr-boxington's exact pin and signed lock entries cover every supported platform.

Mise itself is explicitly excluded from this dependency refresh. Every workflow continues to install exactly mise `2026.9.18` (2026-09-30). Root `min_version` is `{ hard = "2026.9.13", soft = "2026.9.18" }`:
Homebrew's `mise` formula was 2026.9.15 on 2026-10-01, so a hard 2026.9.18
floor would reject a package-manager installation, and lockfile behavior was
measured identical from 2026.9.13 through 2026.9.18. The locks stay at lockfile
revision 1 so that floor can read them; see
[dependency and release updates](development.md#dependency-and-release-updates).
Raise mise in its own change, updating these together: the root `mise.toml`
`min_version`; every `jdx/mise-action` `version:` input in the bundle-build, CI,
quality, native-tests and release workflows; `WORKFLOW_MISE_VERSION` in
`packages/kuru-delivery/src/published_windows.rs`; the delivery tests
`tests/release_workflow.rs`, `tests/support/mise_acceptance.rs` and the
`tests/repo_validation.rs` workflow fixtures; the mise CLI citations in
`src/repo/workflows.rs` after rechecking them; every `mise.lock` through the
documented refresh; and the minimum mise version stated in development and
installation docs.

Cospec is `0.8.3`, confirmed by its [GitHub release](https://github.com/aligned-team/cospec/releases/tag/v0.8.3). Its standalone executable embeds its supported OpenSpec version, 1.13.1 for this release. No project OpenSpec, Bun or Python dependency is required. The standalone gate contract passes without the former compatibility preload, which has been removed; see [development](development.md).

`cargo-audit` is scoped to delivery's advisory quality tasks. It scans only the
current workspace root `Cargo.lock` against an explicitly refreshed RustSec database;
the ordinary task runs offline with database fetch and yanked-package index
checks disabled. It does not enforce licenses, source policy or general
dependency bans. The package's audit configuration has no ignored advisory IDs:
an advisory failure needs a dependency correction, or a separate reviewed
exception with the exact RustSec identifier and rationale.

The scanner runs from delivery's owned `.cargo/audit.toml` and replaces any
inherited Cargo Home with an empty temporary directory. This isolates
cargo-audit configuration lookup so a user's Cargo Home audit policy cannot
change the repository scan.

The delivery task installs the Rust source tool through mise's Cargo backend at
the exact `0.22.2` pin with Cargo's locked resolution and default features
disabled; see [mise's Cargo backend options](https://mise.jdx.dev/dev-tools/backends/cargo.html#default-features).
Refresh the public database in an isolated absolute directory, then scan it:

```sh
mise run //packages/kuru-delivery:setup:advisories
KURU_ADVISORY_DB=/absolute/private/kuru-rustsec-db \
  mise run //packages/kuru-delivery:audit:advisories:refresh
KURU_ADVISORY_DB=/absolute/private/kuru-rustsec-db \
  mise run //packages/kuru-delivery:audit:advisories
```

The refresh command records the checked detached RustSec revision and commit
time. The offline scan rejects a dirty, foreign, non-detached, or database
revision older than 90 days; refresh it again rather than bypassing the
freshness check. The database is public advisory data, never a project or user
credential store.

The docs app selects npm `12.2.0` separately because Node `26.10.0` bundles
npm `11.19.1`. Its app-owned mise alias retains npm's PATH priority over Node's
bundled copy, and the tool option pins the official tarball's SHA-512 digest.
The npm backend does not supply per-platform archive URLs or attestations in
mise.lock; its exact version and explicit checksum remain authoritative.
[npm's declared Node range](https://registry.npmjs.org/npm/12.2.0) includes
26.10.0. A clean locked installation selects these versions; dependency lifecycle
scripts retain npm 12's default-deny policy, including optional `fsevents`.

## CI actions

Actions retain their previously reviewed immutable release commits; this refresh audits Cargo and mise-declared dependencies.

| Action                      | Release                                                                      | Commit                                     |
| --------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------ |
| `actions/checkout`          | [`v7.0.1`](https://github.com/actions/checkout/releases/tag/v7.0.1)          | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
| `jdx/mise-action`           | [`v4.3.0`](https://github.com/jdx/mise-action/releases/tag/v4.3.0)           | `c2a87611a18de5b3828c5652fe268e992400cb5c` |
| `actions/upload-artifact`   | [`v7.0.1`](https://github.com/actions/upload-artifact/releases/tag/v7.0.1)   | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` |
| `actions/download-artifact` | [`v8.0.1`](https://github.com/actions/download-artifact/releases/tag/v8.0.1) | `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` |
| `Swatinem/rust-cache`       | [`v2.9.2`](https://github.com/Swatinem/rust-cache/releases/tag/v2.9.2)       | `6323deb102c322ba6fcbdcafc7e3dddab59af2b6` |

The reqwest 0.13 upgrade changes its TLS feature name to `rustls`; the capability filesystem crates move together to 4.0.3. Rust 1.99.0 is shared by mise and rust-toolchain.toml. Re-run format, lint, protocol tests and coverage after future updates instead of assuming minor version changes are compatible.

## Upstream transitive constraints

All direct Rust pins are the latest stable releases. Cargo's full update resolves
transitives to the latest compatible releases. Latest Axum 0.8.9 pins
`matchit = 0.8.4` exactly, preventing 0.8.6; a `cargo update --precise` probe
confirmed this constraint. The lockfile retains the supported resolution rather
than overriding an upstream requirement with an unreviewed fork.

Ratatui enables only its Crossterm 0.29 backend and layout cache. Disabling
unused default widgets/backends removed the older optional termwiz/sha2 and
pinned generic-array chain from the lockfile.

OpenAI authentication and model requests use Kuru's Rust connector package and
the existing HTTP, cryptography and private-filesystem dependencies. Codex CLI
and app-server are no longer runtime or development-tool prerequisites for
these providers. No separate harness is installed or bundled. Node and npm
remain scoped to the documentation app.
