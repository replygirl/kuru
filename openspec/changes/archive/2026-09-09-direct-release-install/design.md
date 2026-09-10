## Context

See the proposal and repository-delivery delta for scope. Existing release
archives contain three root entries: kuru, LICENSE and README.md. The existing
shell entrypoint compiles the Rust delivery helper for binary installation, while
the installed executable already performs native updates without Cargo.

## Goals / Non-Goals

Keep the first bootstrap small and portable to system Bash 3.2 on macOS and
Bash on Linux. Preserve the package-owned delivery boundary, archive format and
native updater. No new package manager, runtime, binary asset, release trigger,
authentication system or background updater is needed.

## Decisions

Own the bootstrap at packages/kuru-delivery/support/install.sh. Keep --source in
the existing scripts/install.sh and forward its binary-install mode to that
bootstrap. This removes compiler requirements without duplicating source builds.

With no explicit version, fetch latest/download/SHA256SUMS once, require one
valid matching host archive, and derive its explicit version. Download from that
version's immutable release path using the selected digest. This avoids both a
JSON-parser dependency and a race between two mutable latest URLs. Explicit
versions use their own manifest. Retain --release-base/KURU_RELEASE_BASE for
HTTPS mirrors and local offline directories, --install-dir/KURU_INSTALL_DIR
for destinations, and --target for any of the four supported archive targets.
CLI values take precedence over environment values. A custom base remains a
literal version directory and requires --version, matching the existing contract;
only default GitHub latest resolution derives an immutable version URL.

Use HTTPS-only curl downloads and enforce streaming limits of 64 KiB for the
manifest and 128 MiB each for compressed archive and expanded tar, matching the
native installer. Apply the same limits to local sources. Use checksum
verification, bounded gzip decompression and standard tar inspection. Require the known flat regular
entries without duplicates, then stream only kuru to an already-created private
staging file. Never extract arbitrary members into the filesystem or run the
downloaded executable during bootstrap. Stage beside the destination, reject
symlink/directory destinations, set executable permissions and rename atomically.
Traps clean temporary material on success, error or interruption; test real
termination during a blocked download as well as a transport error return.

Bound the final stdout-only executable extraction too: sparse metadata can describe
more logical output than the bounded physical tar stream. The shell boundary validates logical tar entries for safe fixed-member extraction;
it does not reproduce the Rust updater's raw-header/PAX parser. Do not claim those
checks are identical. The release source remains the trust anchor; checksums
provide corruption detection. ShellCheck and real Bash integration tests cover
the bootstrap, while existing native archive tests remain unchanged.

README order is mise, shell, source. Explain activation and exact pinning, with
mise's release-age behavior in the detailed guide as needed. Use normal GitHub
clone and release links. Update both installation guides and linked release text
to remove repository-visibility commentary, preserving platform facts such as
Ubuntu 24.04/glibc requirements and explicit update behavior. Mise-managed
binaries update through mise rather than modifying its installation cache.

## Operational surface

Only user-invoked installation writes a selected binary directory. Supported
targets remain Linux and macOS on x86_64 and arm64. No daemon, listening port,
credential, remote write or deployment workflow is added. The docs site continues
to deploy only at the end of Release, from its released source.

## Integration contract

GitHub supplies SHA256SUMS and kuru-VERSION-TARGET.tar.gz. The manifest selects
exactly one 64-hex digest and validated version for the current target. Reject
malformed/ambiguous data and insecure transport. Mise's GitHub backend discovers
the same archives and root executable; prove the actual release installation in
isolated mise configuration/data/cache without changing a user's active tools.

## Risks / Trade-offs

- System tar differences: run the same behavioral suite on macOS and Linux;
  keep extraction stdout-only and use the existing three-entry format.
- Latest can move: freeze the selected manifest identity before downloading.
- A failed install can corrupt an existing app: validate before replacement,
  test interruption/corruption/destination failures, and assert unchanged bytes.
- A fake transport cannot prove hosted delivery: separately verify downloaded
  real release assets and actual mise installation after publication.
