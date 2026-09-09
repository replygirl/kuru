# Dependency audit

Latest stable releases were checked on 2026-09-09 against the official crates.io sparse registry, npm registry, upstream GitHub releases and mise release metadata. Direct dependencies are exactly pinned; lockfiles resolve their compatible transitive dependencies. Prereleases are excluded.

## Rust dependencies

| Dependency | Latest stable pin | Registry |
| --- | --- | --- |
| `anyhow` | `1.0.104` | [sparse index](https://index.crates.io/an/yh/anyhow) |
| `async-trait` | `0.1.92` | [sparse index](https://index.crates.io/as/yn/async-trait) |
| `serde` | `1.0.229` | [sparse index](https://index.crates.io/se/rd/serde) |
| `serde_json` | `1.0.151` | [sparse index](https://index.crates.io/se/rd/serde_json) |
| `toml` | `1.1.5+spec-1.1.0` | [sparse index](https://index.crates.io/to/ml/toml) |
| `rusqlite` | `0.40.2` | [sparse index](https://index.crates.io/ru/sq/rusqlite) |
| `uuid` | `1.26.0` | [sparse index](https://index.crates.io/uu/id/uuid) |
| `sha2` | `0.11.0` | [sparse index](https://index.crates.io/sh/a2/sha2) |
| `tokio` | `1.53.1` | [sparse index](https://index.crates.io/to/ki/tokio) |
| `reqwest` | `0.13.5` | [sparse index](https://index.crates.io/re/qw/reqwest) |
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

## Development tools

| Tool | Stable pin |
| --- | --- |
| `rust` | `1.98.1` |
| `bun` | `1.4.2` |
| Codex (optional live-provider transport) | `0.153.4` |
| `python` | `3.14.7` |
| `github:aligned-team/cospec` | `0.7.0` |
| `aqua:jdx/hk` | `1.58.1` |
| `aqua:tamasfe/taplo` | `0.10.0` |
| `aqua:koalaman/shellcheck` | `0.11.0` |
| `aqua:rhysd/actionlint` | `1.7.12` |
| `aqua:astral-sh/ruff` | `0.16.6` |
| `cargo:cargo-llvm-cov` | `0.9.1` |
| `@fission-ai/openspec` | `1.12.0` |
| CI mise | `2026.9.3` |

Cospec is `0.7.0`, confirmed by both [GitHub releases](https://github.com/aligned-team/cospec/releases/tag/v0.7.0) and [npm](https://www.npmjs.com/package/@aligned-team/cospec). The mise latest endpoint returned an older release during the audit; it was not used to downgrade the verified newer release. OpenSpec is installed locally as the cospec wrapper dependency. Its latest 1.x API is validated by Kuru’s cospec gates.

## CI actions

All actions are pinned to the immutable commit for the latest stable release.

| Action | Release | Commit |
| --- | --- | --- |
| `actions/checkout` | [`v7.0.1`](https://github.com/actions/checkout/releases/tag/v7.0.1) | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
| `jdx/mise-action` | [`v4.3.0`](https://github.com/jdx/mise-action/releases/tag/v4.3.0) | `c2a87611a18de5b3828c5652fe268e992400cb5c` |
| `actions/upload-artifact` | [`v7.0.1`](https://github.com/actions/upload-artifact/releases/tag/v7.0.1) | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` |
| `actions/download-artifact` | [`v8.0.1`](https://github.com/actions/download-artifact/releases/tag/v8.0.1) | `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` |

The reqwest 0.13 upgrade changes its TLS feature name to `rustls`; the capability filesystem crates move together to 4.0.3. Rust 1.98.1 is shared by mise and rust-toolchain.toml. Re-run format, lint, protocol tests and coverage after future updates instead of assuming minor version changes are compatible.

## Upstream transitive constraints

All direct pins are the latest stable releases. Cargo's full update resolves
transitives to the latest compatible releases. Latest Axum 0.8.9 pins
`matchit = 0.8.4` exactly, preventing 0.8.6; a `cargo update --precise` probe
confirmed this constraint. The lockfile retains the supported resolution rather
than overriding an upstream requirement with an unreviewed fork.

Ratatui enables only its Crossterm 0.29 backend and layout cache. Disabling
unused default widgets/backends removed the older optional termwiz/sha2 and
pinned generic-array chain from the lockfile.

Codex 0.153.4 is the latest published npm release, verified via the official
`@openai/codex` registry metadata. It is pinned for development and live provider
verification; offline demo and Responses operation do not require Codex.
