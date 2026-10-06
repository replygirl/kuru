# Design

## Context

`cli::update` is dispatched before workspace paths/configuration/trust and is already a fixed installation operation. Unix release mode calls `archive::install`; source mode still launches the old shell installer. Windows uses its checked helper and receipt engine. The existing platform filesystem boundary retains directories, validates regular single-link inputs/destinations and distinguishes uncertain publication. The first slice must not absorb the later Unix transaction engine.

## Goals / Non-Goals

**Goals:** Put a small ownership refusal ahead of both existing Unix update paths, preserve checked filesystem rules and provide exact no-effect evidence. Follow proposal scope and the P25 ownership ruling.

**Non-Goals:** Recovery receipts, native source-build conversion, Linux replaced-image launch, notices, Homebrew packaging and Windows changes belong to their already assigned later slices.

## Decisions

### Pure component-exact manager classification

Delivery owns `Manager`, captured ownership environment roots and path classification. Resolve the executable first, normalize native aliases consistently with the checked directory boundary, then compare path components against Homebrew Cellar roots and mise installs roots from the existing environment/default conventions. Relative or empty environment roots do not broaden authority. Fixed labels and upgrade hints are data, not subprocess commands. Manager refusal runs before any network or source-build preparation.

Rejected: matching string prefixes, assuming user-writability means Kuru ownership, launching package-manager commands, or treating trusted workspace configuration as update authority.

### Checked unmanaged preflight behind existing boundaries

Return a retained installation parent and held regular executable after manager classification. Existing single-link, no-follow and directory identity checks remain strict. Current effective ownership and parent replacement access are native filesystem mechanics; add only a narrow safe platform preflight if the current API cannot express those facts, using its existing native dependency and no unsafe consumer code. A 0555 executable in a writable parent remains replaceable; a symlink/foreign owner/inaccessible parent refuses. Preserve current archive publication checks. This held capture is a fresh preflight before delegating to current installers; it grants no persistent mutation authority across their network/build work. The later receipt engine will consume and revalidate the retained installation after preparation.

Rejected: a writable-file-open test that rejects valid read-only executables, creating a probe file before refusal, relaxing hard-link checks for destinations, or executing the installed image to discover ownership.

### Call order and bounded integration

After ordinary flag validation, obtain the actual executable and ownership capture, classify and retain/check its installation, then enter the existing release or source path. Only Unix changes. Source mode's legacy shell delegation is intentionally removed in the next receipt slice rather than expanding this one. Existing success/stdout behavior and fixed config-free dispatch remain intact; manager refusals go through ordinary safe stderr error handling.

The first slice updates install/command documentation and the authored AGENTS convention only where required by the new ownership rule. It does not add release workflow, schema, private memory or protocol changes. The same manager labels/hints will be reused by the later notice module.

## Integration contract

Inputs are the actual running executable's resolved filesystem path and a one-invocation snapshot of HOME/XDG/mise/Homebrew root overrides. Delivery owns manager classification and fixed upgrade hints; the platform boundary owns retained regular-file/directory identity and effective native access checks. The CLI does not invoke either package manager. Output is either a retained checked installation or an ordinary stderr refusal before existing release/source work. No receipt/schema/wire or SDK contract changes, and no workspace configuration participates. Native fixtures copy the actual executable into isolated manager-shaped paths, keep child output bounded/drained and prove request/build counters stay zero; no test changes runner-global environment.

## Operational surface

The existing local `kuru update --version` and `--source` commands keep their flags. No listener, container, credential, background process, dependency version or installation layout is introduced. Tests run native copied executable fixtures under package-owned mise tasks; Windows-target checks verify compilation only, and unchanged Windows receipt/bootstrap behavior remains native CI evidence for normal integrated delivery.

## Risks / Trade-offs

- [Environment-defined manager roots can be absent or noncanonical] → Require absolute nonempty roots, normalize aliases and use component-exact matching; test defaults, overrides and near misses.
- [A destination can change after initial inspection] → Retain its checked parent/file and preserve revalidation and existing final publication rejection; do not treat an initial pathname string as durable authority.
- [Other managers have different layouts] → Recognize the settled mise/Homebrew policy only; generic current-user/checked-file protections still apply elsewhere, without claiming universal package-manager detection.
- [Cross-target compilation is not native acceptance] → Record actual host child/filesystem behavior separately and leave supported-platform CI/previous-release acceptance to the appropriate integrated head.
