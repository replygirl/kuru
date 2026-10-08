# Patched terminal dependency

`crossterm-0.29.0/` contains the published crates.io crossterm 0.29.0 archive,
retaining its MIT license, manifest, original lockfile and VCS metadata. UTF-8
text uses this repository's LF checkout convention.

- Upstream archive: `https://static.crates.io/crates/crossterm/crossterm-0.29.0.crate`
- Archive SHA256: `d8b9f2e4c67f833b660cdb0a3523065869fb35570177239812ed4c905aeff87b`
- Upstream commit: `36d95b26a26e64b0f8c12edfe11f410a6d56a812`

The only source correction is in `src/event/sys/windows/parse.rs`: a native
`VK_MENU` key-up carrying committed Alt-code text emits a character Press.
Ordinary character key-up still emits Release. Actual Windows ConPTY acceptance
checks exact decomposed Unicode bytes, distinct ordinary releases and persisted
recall transcripts. Kuru's release filtering and Unix event parsing are unchanged.

Root `[patch.crates-io]` selects this exact-version dependency for every consumer,
including ratatui. It is excluded from Kuru workspace membership and covered by
the same foreign-dependency reporting boundary as its original registry source;
all Kuru application modules remain in their existing coverage inventory.
The original published manifest is excluded from Kuru's TOML reformatting;
every Kuru-owned manifest remains checked.

Compare this directory with the verified upstream archive after LF normalization
when changing the patch. Remove the patch and this directory once a verified,
exactly pinned upstream release includes the correction. Follow the dependency
refresh procedure in [development](../../../docs/development.md#dependency-and-release-updates).
