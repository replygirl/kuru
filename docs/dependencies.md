# Dependency audit

Releases were checked on 2026-09-10 against the official crates.io sparse registry, npm registry, upstream GitHub releases and mise release metadata. Direct dependencies are exactly pinned; lockfiles resolve their compatible transitive dependencies. Rust dependencies use stable releases. The docs app follows cospec's VitePress 2 preview architecture and pins the latest available alpha explicitly.

The Dolt change checked SQLx 0.9.0, UUID 1.26.1 and full Dolt 2.3.3 on
2026-09-10. The memory package's catalog records the measured native archive,
executable and license digests for each cataloged target. The Windows change
also pins the native API bindings, strict ZIP codec and ConPTY test driver below.
The offline tokenizer addition checked `tiktoken-rs` 0.12.0 on 2026-09-22;
its embedded `o200k_base.tiktoken` SHA-256 matches [OpenAI's published
asset hash](https://github.com/openai/tiktoken/blob/main/tiktoken_ext/openai_public.py),
`446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`.
The connector package carries the crate and asset MIT attributions in
`THIRD_PARTY_NOTICES.md`.

## Rust dependencies

| Dependency | Latest stable pin | Registry |
| --- | --- | --- |
| `anyhow` | `1.0.104` | [sparse index](https://index.crates.io/an/yh/anyhow) |
| `async-trait` | `0.1.92` | [sparse index](https://index.crates.io/as/yn/async-trait) |
| `serde` | `1.0.229` | [sparse index](https://index.crates.io/se/rd/serde) |
| `serde_json` | `1.0.151` | [sparse index](https://index.crates.io/se/rd/serde_json) |
| `toml` | `1.1.6+spec-1.1.0` | [sparse index](https://index.crates.io/to/ml/toml) |
| `rusqlite` | `0.40.2` | [sparse index](https://index.crates.io/ru/sq/rusqlite) |
| `uuid` | `1.26.1` | [sparse index](https://index.crates.io/uu/id/uuid) |
| `sqlx` | `0.9.0` | [sparse index](https://index.crates.io/sq/lx/sqlx) |
| `sha2` | `0.11.0` | [sparse index](https://index.crates.io/sh/a2/sha2) |
| `tokio` | `1.53.1` | [sparse index](https://index.crates.io/to/ki/tokio) |
| `reqwest` | `0.13.5` | [sparse index](https://index.crates.io/re/qw/reqwest) |
| `tiktoken-rs` | `0.12.0` | [sparse index](https://index.crates.io/ti/kt/tiktoken-rs) |
| `futures` | `0.3.34` | [sparse index](https://index.crates.io/fu/tu/futures) |
| `axum` | `0.8.9` | [sparse index](https://index.crates.io/ax/um/axum) |
| `tower` | `0.5.3` | [sparse index](https://index.crates.io/to/we/tower) |
| `ratatui` | `0.30.2` | [sparse index](https://index.crates.io/ra/ta/ratatui) |
| `crossterm` | `0.29.0` | [sparse index](https://index.crates.io/cr/os/crossterm) |
| `clap` | `4.6.6` | [sparse index](https://index.crates.io/cl/ap/clap) |
| `tempfile` | `3.27.0` | [sparse index](https://index.crates.io/te/mp/tempfile) |
| `unicode-width` | `0.2.2` | [sparse index](https://index.crates.io/un/ic/unicode-width) |
| `url` | `2.5.8` | [sparse index](https://index.crates.io/3/u/url) |
| `cap-std` | `4.0.3` | [sparse index](https://index.crates.io/ca/p-/cap-std) |
| `cap-fs-ext` | `4.0.3` | [sparse index](https://index.crates.io/ca/p-/cap-fs-ext) |
| `nix` | `0.31.3` | [sparse index](https://index.crates.io/3/n/nix) |
| `base64` | `0.23.1` | [sparse index](https://index.crates.io/ba/se/base64) |
| `rustix` | `1.1.4` | [sparse index](https://index.crates.io/ru/st/rustix) |
| `vt100` | `0.16.2` | [sparse index](https://index.crates.io/vt/10/vt100) |
| `tar` | `0.4.46` | [sparse index](https://index.crates.io/3/t/tar) |
| `flate2` | `1.1.10` | [sparse index](https://index.crates.io/fl/at/flate2) |
| `zip` | `8.6.0` | [sparse index](https://index.crates.io/3/z/zip) |
| `crc32fast` | `1.5.1` | [sparse index](https://index.crates.io/cr/c3/crc32fast) |
| `windows-sys` | `0.61.2` | [sparse index](https://index.crates.io/wi/nd/windows-sys) |
| `portable-pty` (Windows terminal tests) | `0.9.0` | [sparse index](https://index.crates.io/po/rt/portable-pty) |
| `scraper` | `0.27.0` | [sparse index](https://index.crates.io/sc/ra/scraper) |
| `roxmltree` | `0.21.1` | [sparse index](https://index.crates.io/ro/xm/roxmltree) |

## Development tools

| Tool | Stable pin |
| --- | --- |
| `rust` | `1.98.1` |
| `github:aligned-team/cospec` | `0.7.1` |
| `aqua:jdx/hk` | `2.2.0` |
| `aqua:tamasfe/taplo` | `0.10.0` |
| `aqua:koalaman/shellcheck` | `0.11.0` |
| `aqua:rhysd/actionlint` | `1.7.12` |
| `cargo:cargo-llvm-cov` | `0.9.1` |
| `mr-boxington` | `1.17.0` |
| `cargo:cargo-audit` (delivery package) | `0.22.2` |
| Node (docs app only) | `26.8.2` |
| npm (docs app only) | `12.0.2` |
| VitePress (docs app) | `2.0.0-alpha.20` |
| `vitepress-plugin-llms` (docs app) | `1.13.5` |
| `oxfmt` (docs app) | `0.67.0` |
| `oxlint` (docs app) | `1.82.0` |
| Cocogitto (delivery package) | `7.0.0` |
| Communiqué (delivery package) | `1.3.5` |
| mise (root `min_version` and CI) | `2026.9.4` |

mr-boxington's exact pin and signed lock entries cover every supported platform.

Cospec is `0.7.1`, confirmed by its [GitHub release](https://github.com/aligned-team/cospec/releases/tag/v0.7.1). Its standalone executable embeds its supported OpenSpec version. No project OpenSpec, Bun or Python dependency is required. The task-scoped compatibility preload remains necessary; see [development](development.md).

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

The docs app selects npm `12.0.2` separately because Node `26.8.2` bundles
npm `11.19.1`. Its app-owned mise alias retains npm's PATH priority over Node's
bundled copy, and the tool option pins the official tarball's SHA-512 digest.
The npm backend does not supply per-platform archive URLs or attestations in
mise.lock; its exact version and explicit checksum remain authoritative.
[npm's declared Node range](https://registry.npmjs.org/npm/12.0.2) includes
26.8.2. A clean locked installation selects these versions; dependency lifecycle
scripts retain npm 12's default-deny policy, including optional `fsevents`.

## CI actions

All actions are pinned to the immutable commit for the latest stable release.

| Action | Release | Commit |
| --- | --- | --- |
| `actions/checkout` | [`v7.0.1`](https://github.com/actions/checkout/releases/tag/v7.0.1) | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
| `jdx/mise-action` | [`v4.3.0`](https://github.com/jdx/mise-action/releases/tag/v4.3.0) | `c2a87611a18de5b3828c5652fe268e992400cb5c` |
| `actions/upload-artifact` | [`v7.0.1`](https://github.com/actions/upload-artifact/releases/tag/v7.0.1) | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` |
| `actions/download-artifact` | [`v8.0.1`](https://github.com/actions/download-artifact/releases/tag/v8.0.1) | `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` |
| `Swatinem/rust-cache` | [`v2.9.2`](https://github.com/Swatinem/rust-cache/releases/tag/v2.9.2) | `6323deb102c322ba6fcbdcafc7e3dddab59af2b6` |

The reqwest 0.13 upgrade changes its TLS feature name to `rustls`; the capability filesystem crates move together to 4.0.3. Rust 1.98.1 is shared by mise and rust-toolchain.toml. Re-run format, lint, protocol tests and coverage after future updates instead of assuming minor version changes are compatible.

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
