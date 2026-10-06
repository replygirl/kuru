#!/usr/bin/env bash
set -euo pipefail

kuru_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fail() { printf 'kuru: %s\n' "$*" >&2; exit 1; }
# Bootstrap is compiler-free: preserve pending native evidence without parsing
# or executing an image. Native recovery owns the strict transaction lock;
# these two shell checks retain the documented shell race limitation.
check_pending_update() {
  local pending="$kuru_directory/.kuru-update" field mode links retained before after
  [[ -e $pending || -L $pending ]] || return 0
  [[ ! -L $pending && -d $pending && -O $pending ]] || fail 'pending update directory is unsafe; state retained'
  command -v stat >/dev/null || fail 'stat is required to inspect pending update state'
  mode=$(stat -c '%a' -- "$pending" 2>/dev/null) || mode=$(stat -f '%Lp' "$pending")
  [[ $mode == 700 ]] || fail 'pending update directory must be private; state retained'
  if [[ ! -e $pending/receipt.json && ! -L $pending/receipt.json && ! -e $pending/receipt.next && ! -L $pending/receipt.next ]]; then
    for field in "$pending"/* "$pending"/.[!.]* "$pending"/..?*; do
      [[ -e $field || -L $field ]] || continue
      [[ $field == "$pending/install.lock" && ! -L $field && -f $field && -O $field ]] || fail 'unknown pending update evidence; state retained'
      mode=$(stat -c '%a:%h:%s' -- "$field" 2>/dev/null) || mode=$(stat -f '%Lp:%l:%z' "$field")
      [[ $mode == 600:1:0 ]] || fail 'pending update lock is unsafe; state retained'
    done
    return 0
  fi
  for field in "$pending/receipt.json" "$pending/receipt.next"; do
    [[ -e $field || -L $field ]] || continue
    [[ ! -L $field && -f $field && -O $field ]] || fail 'pending receipt must be an owned regular file; state retained'
    mode=$(stat -c '%a:%h:%s' -- "$field" 2>/dev/null) || mode=$(stat -f '%Lp:%l:%z' "$field")
    [[ ${mode%:*} == 600:1 ]] || fail 'pending receipt permissions or links are unsafe; state retained'
    links=${mode##*:}
    (( links <= 65536 )) || fail 'pending receipt exceeds bounds; state retained'
  done
  [[ ! -e $kuru_directory/kuru && ! -L $kuru_directory/kuru ]] || fail 'pending update requires one ordinary Kuru run before installation; state retained'
  before=$(stat -c '%d:%i' -- "$pending" 2>/dev/null) || before=$(stat -f '%d:%i' "$pending")
  retained=$(mktemp -d "$kuru_directory/.kuru-update.abandoned.XXXXXX")
  mv -- "$pending" "$retained/pending" || fail 'could not preserve pending update state'
  after=$(stat -c '%d:%i' -- "$retained/pending" 2>/dev/null) || after=$(stat -f '%d:%i' "$retained/pending")
  [[ $before == "$after" && ! -L $retained/pending && -d $retained/pending && -O $retained/pending ]] || fail 'pending update changed while being preserved; files retained'
  printf 'kuru: pending update state preserved at %q; installing only the requested image.\n' "$retained/pending" >&2
}
if [[ "${1:-}" == "--source" ]]; then
  shift
  if (( $# != 0 )); then
    echo "Source installation takes no arguments; set KURU_INSTALL_DIR for the destination." >&2
    exit 2
  fi
  kuru_install_dir=${KURU_INSTALL_DIR:-"${HOME}/.local/bin"}
  kuru_directory=$kuru_install_dir
  check_pending_update
  # Source installation needs the pinned compiler and bundled engine input.
  # Repository hook installation and maintainer-only tools, including the
  # mr-boxington build cache, belong to setup.
  export KURU_MBX=0
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
  check_pending_update
  mv -f -- "$kuru_staging" "$kuru_install_dir/kuru"
  "$kuru_install_dir/kuru" --version
  echo "Installed at $kuru_install_dir/kuru; add $kuru_install_dir to PATH."
else
  exec bash "$kuru_repo/packages/kuru-delivery/support/install.sh" "$@"
fi
