# Dependency audit

Releases were checked on 2026-09-09 against the official crates.io sparse registry, npm registry, upstream GitHub releases and mise release metadata. Direct dependencies are exactly pinned; lockfiles resolve their compatible transitive dependencies. Rust dependencies use stable releases. The docs app follows cospec's VitePress 2 preview architecture and pins the latest available alpha explicitly.

The Dolt change checked SQLx 0.9.0, UUID 1.26.1 and full Dolt 2.3.3 on
2026-09-10. The memory package's catalog records the measured native archive,
executable and license digests for all four supported targets.

## Rust dependencies

| Dependency | Latest stable pin | Registry |
| --- | --- | --- |
| `anyhow` | `1.0.104` | [sparse index](https://index.crates.io/an/yh/anyhow) |
| `async-trait` | `0.1.92` | [sparse index](https://index.crates.io/as/yn/async-trait) |
| `serde` | `1.0.229` | [sparse index](https://index.crates.io/se/rd/serde) |
| `serde_json` | `1.0.151` | [sparse index](https://index.crates.io/se/rd/serde_json) |
| `toml` | `1.1.5+spec-1.1.0` | [sparse index](https://index.crates.io/to/ml/toml) |
| `rusqlite` | `0.40.2` | [sparse index](https://index.crates.io/ru/sq/rusqlite) |
| `uuid` | `1.26.1` | [sparse index](https://index.crates.io/uu/id/uuid) |
| `sqlx` | `0.9.0` | [sparse index](https://index.crates.io/sq/lx/sqlx) |
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
| `base64` | `0.23.1` | [sparse index](https://index.crates.io/ba/se/base64) |
| `rustix` | `1.1.4` | [sparse index](https://index.crates.io/ru/st/rustix) |
| `vt100` | `0.16.2` | [sparse index](https://index.crates.io/vt/10/vt100) |
| `tar` | `0.4.46` | [sparse index](https://index.crates.io/3/t/tar) |
| `flate2` | `1.1.10` | [sparse index](https://index.crates.io/fl/at/flate2) |
| `scraper` | `0.27.0` | [sparse index](https://index.crates.io/sc/ra/scraper) |
| `roxmltree` | `0.21.1` | [sparse index](https://index.crates.io/ro/xm/roxmltree) |

## Development tools

| Tool | Stable pin |
| --- | --- |
| `rust` | `1.98.1` |
| Codex (optional live-provider transport) | `0.153.4` |
| `github:aligned-team/cospec` | `0.7.0` |
| `aqua:jdx/hk` | `1.58.1` |
| `aqua:tamasfe/taplo` | `0.10.0` |
| `aqua:koalaman/shellcheck` | `0.11.0` |
| `aqua:rhysd/actionlint` | `1.7.12` |
| `cargo:cargo-llvm-cov` | `0.9.1` |
| Node (docs app only) | `26.8.2` |
| npm (bundled with Node) | `11.19.1` |
| VitePress (docs app) | `2.0.0-alpha.20` |
| `vitepress-plugin-llms` (docs app) | `1.13.5` |
| `oxfmt` (docs app) | `0.67.0` |
| `oxlint` (docs app) | `1.82.0` |
| Cocogitto (delivery package) | `7.0.0` |
| Communiqué (delivery package) | `1.3.5` |
| CI mise | `2026.9.3` |

Cospec is `0.7.0`, confirmed by both [GitHub releases](https://github.com/aligned-team/cospec/releases/tag/v0.7.0) and [npm](https://www.npmjs.com/package/@aligned-team/cospec). The mise latest endpoint returned an older release during the audit; it was not used to downgrade the verified newer release. Its standalone executable embeds its supported OpenSpec version. No project OpenSpec, Bun or Python dependency is required.

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
