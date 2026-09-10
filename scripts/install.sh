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
  cargo build --manifest-path "$kuru_repo/Cargo.toml" --release --locked -p kuru
  mkdir -p -- "$kuru_install_dir"
  if [[ -L "$kuru_install_dir/kuru" || -d "$kuru_install_dir/kuru" ]]; then
    echo "Refusing to replace a symlink or directory at the install destination." >&2
    exit 1
  fi
  kuru_staging=$(mktemp "$kuru_install_dir/.kuru-install.XXXXXX")
  trap 'rm -f -- "$kuru_staging"' EXIT
  install -m 755 "${CARGO_TARGET_DIR:-$kuru_repo/target}/release/kuru" "$kuru_staging"
  mv -f -- "$kuru_staging" "$kuru_install_dir/kuru"
  "$kuru_install_dir/kuru" --version
  echo "Installed at $kuru_install_dir/kuru; add $kuru_install_dir to PATH."
else
  exec bash "$kuru_repo/packages/kuru-delivery/support/install.sh" "$@"
fi
