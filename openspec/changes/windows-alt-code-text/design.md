# Design

## Context

The actual pre-composer receipt retains U+0301 only as `Char/Release/NONE`, while ordinary characters have Press/Release pairs. The upstream parser still knows the original `VK_MENU` text-commit record, but its public KeyEvent no longer exposes that discriminator. A UI release exception would admit indistinguishable ordinary releases and duplicate input.

## Decisions

Patch the existing dependency discriminator to emit Press for committed Alt-code text, retaining ordinary Release. Keep exact version 0.29.0, verified upstream source/license and one root Cargo patch under the TUI app. Exclude the third-party crate from Kuru workspace membership, retaining existing consumer unsafe policies and application inventory. Existing actual ConPTY event and persisted transcript regressions verify the linked parser; do not introduce another input backend or a copied decoder test harness.

## Operational surface

This is the existing local Windows terminal flow on x64 and ARM, with the same Kuru executable and crossterm 0.29.0 version. It introduces no endpoint, credentials, external service, process limit or deployment topology. Native CI enters the public terminal session, sends synthetic UTF-8 bytes through its owned ConPTY and restores modes/ownership before comparing receipts. Unix terminal decoding is unchanged.

## Integration contract

The owned TUI vendor directory contains the verified crates.io source and MIT license. Root Cargo resolution selects that one exact-version source for both Kuru and ratatui. The parser's native Alt-code discriminator becomes a committed character Press; the public KeyEvent shape and ordinary key-up Release contract stay intact. Actual raw receipts must contain one accent Press and normal release pairs, and all seven persisted prompts remain byte-exact.

The pinned coverage tool automatically omits registry/git dependency sources, but a patched local path is not classified the same way. Apply one exact versioned foreign-directory filter before all canonical LCOV/full-JSON/summary exports and local workspace reporting. Retain default filters, instrumentation, JSON/export self-checks, every application source and all 95% gates. A dummy Cargo source-replacement configuration would misrepresent the actual dependency resolution.

## Risks / Trade-offs

Maintaining a dependency patch requires tracking the original archive hash and minimal source difference. Document that provenance and remove the patch when an exact verified upstream release contains the same correction. Native runtime checks cannot execute on the macOS host; keep their observed original failure and require corrected native acceptance before merge.
