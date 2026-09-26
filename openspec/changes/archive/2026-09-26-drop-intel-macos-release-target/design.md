## Context

The implementation shipped in the `runner-images` change of the same PR; see
its archived tasks and verification. This record carries the requirement
wording and the user-facing documentation.

## Goals / Non-Goals

**Goals:** make the canonical specs describe the catalog-derived archive count
and the Intel refusal the code already enforces.

**Non-Goals:** Windows arm64, pruning `macos-x64` entries from `mise.lock`, and
any change to past published releases.

## Decisions

- Phrase counts as "one per catalog target" rather than replacing "five" with
  "four". The rejected alternative, a new literal, would drift again on the next
  catalog change, while `release.rs` already derives counts from the catalog.
- Record the refusal as a new requirement instead of modifying the long
  "Verified release installation and update" requirement, so that requirement
  and its scenarios stay byte-identical.
- Keep the refusal message naming v0.9.0 and put the installation pointer in
  the docs. Rejected: a longer refusal with the raw URL, which would widen the
  pinned test string and duplicate the docs.

## Operational surface

- Runners: Release builds on `macos-latest` (Apple Silicon), `ubuntu-latest`,
  `ubuntu-24.04-arm` and `windows-2025`; no Intel macOS runner remains.
- Binary arches published: `aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`,
  `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`.
- No bind address, container, secret or connection-limit change.

## Risks / Trade-offs

- [Intel users on v0.9.0 run `kuru update` and find no matching asset] →
  the `feat(release)!:` marker makes the release a breaking bump flagged in the
  notes, and the installation docs name the v0.9.0 path.
