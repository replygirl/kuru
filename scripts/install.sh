#!/usr/bin/env bash
set -euo pipefail

kuru_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if [[ "${1:-}" == "--source" ]]; then
  shift
  if (( $# != 0 )); then
    echo "Source installation takes no arguments; set KURU_INSTALL_DIR for the destination." >&2
    exit 2
  fi
  kuru_install_dir=${KURU_INSTALL_DIR:-"${HOME}/.local/bin"}
  # Source installation needs the pinned compiler and bundled engine input.
  # Repository hook installation and maintainer-only tools belong to setup.
  MISE_NO_HOOKS=1 mise -C "$kuru_repo" install rust
  kuru_host_target=$(mise -C "$kuru_repo" exec rust -- rustc --print host-tuple)
  if [[ -n "${CARGO_BUILD_TARGET:-}" && "$CARGO_BUILD_TARGET" != host && "$CARGO_BUILD_TARGET" != "$kuru_host_target" ]]; then
    echo "Source installation runs on $kuru_host_target; use the build task to cross-compile for $CARGO_BUILD_TARGET." >&2
    exit 2
  fi
  MISE_TASK_RUN_AUTO_INSTALL=false mise -C "$kuru_repo" run //apps/kuru-tui:build:release -- --target host
  kuru_target_dir=${CARGO_TARGET_DIR:-"$kuru_repo/target"}
  if [[ "$kuru_target_dir" != /* ]]; then
    kuru_target_dir="$kuru_repo/$kuru_target_dir"
  fi
  mkdir -p -- "$kuru_install_dir"
  if [[ -L "$kuru_install_dir/kuru" || -d "$kuru_install_dir/kuru" ]]; then
    echo "Refusing to replace a symlink or directory at the install destination." >&2
    exit 1
  fi
  kuru_staging=$(mktemp "$kuru_install_dir/.kuru-install.XXXXXX")
  trap 'rm -f -- "$kuru_staging"' EXIT
  install -m 755 "$kuru_target_dir/release/kuru" "$kuru_staging"
  mv -f -- "$kuru_staging" "$kuru_install_dir/kuru"
  "$kuru_install_dir/kuru" --version
  echo "Installed at $kuru_install_dir/kuru; add $kuru_install_dir to PATH."
else
  exec bash "$kuru_repo/packages/kuru-delivery/support/install.sh" "$@"
fi
