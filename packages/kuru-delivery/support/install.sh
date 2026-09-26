#!/usr/bin/env bash
# Compiler-free bootstrap for the same release archives used by the native updater.
set -euo pipefail
export LC_ALL=C
unset TAR_OPTIONS GZIP
umask 077

kuru_version=''
kuru_target=''
kuru_base=${KURU_RELEASE_BASE:-}
kuru_directory=${KURU_INSTALL_DIR:-}
kuru_stage=''
kuru_producer=''
kuru_consumer=''
kuru_archive_limit=$((128 * 1024 * 1024))
kuru_manifest_limit=$((64 * 1024))
kuru_support_limit=$((4 * 1024 * 1024))
kuru_support_member_limit=$((512 * 1024))
kuru_version_pattern='^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$'

fail() {
  printf 'kuru: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  kuru_status=$?
  trap - EXIT INT TERM HUP
  # These are disposable direct children, never a pipeline's hidden processes.
  for kuru_pid in "$kuru_producer" "$kuru_consumer"; do
    if [[ -n $kuru_pid ]]; then
      kill -KILL "$kuru_pid" 2>/dev/null || true
      wait "$kuru_pid" 2>/dev/null || true
    fi
  done
  if [[ -n $kuru_stage ]]; then
    rm -rf -- "$kuru_stage"
  fi
  exit "$kuru_status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

while (( $# )); do
  kuru_option=${1%%=*}
  case "$kuru_option" in
    --help|-h)
      cat <<'KURU_HELP'
Install Kuru's verified native release without a compiler.

Usage: bash install.sh [--version VERSION] [--target TARGET]
                       [--release-base HTTPS_URL_OR_DIRECTORY]
                       [--install-dir DIRECTORY]

Defaults to the latest GitHub release for this host and ~/.local/bin.
KURU_INSTALL_DIR and KURU_RELEASE_BASE supply defaults for their options.
A custom release base is a literal version directory and requires --version.
KURU_HELP
      exit 0
      ;;
    --version|--target|--release-base|--install-dir)
      if [[ $1 == *=* ]]; then
        kuru_value=${1#*=}
        shift
      else
        (( $# >= 2 )) || fail "$1 requires a value"
        kuru_value=$2
        shift 2
      fi
      [[ -n $kuru_value ]] || fail "$kuru_option requires a value"
      case "$kuru_option" in
        --version) kuru_version=$kuru_value ;;
        --target) kuru_target=$kuru_value ;;
        --release-base) kuru_base=$kuru_value ;;
        --install-dir) kuru_directory=$kuru_value ;;
      esac
      ;;
    *) fail "unknown option: $1 (see --help)" ;;
  esac
done

if [[ -n $kuru_version ]]; then
  kuru_version=${kuru_version#v}
  [[ $kuru_version =~ $kuru_version_pattern ]] || fail 'version must be an explicit semantic version, for example 0.1.0'
fi
if [[ -z $kuru_target ]]; then
  case "$(uname -s):$(uname -m)" in
    Darwin:arm64) kuru_target=aarch64-apple-darwin ;;
    Darwin:x86_64)
      # A Rosetta-translated shell on Apple Silicon also reports x86_64.
      if [[ $(sysctl -n sysctl.proc_translated 2>/dev/null) == 1 ]]; then
        kuru_target=aarch64-apple-darwin
      else
        kuru_target=x86_64-apple-darwin
      fi
      ;;
    Linux:aarch64|Linux:arm64) kuru_target=aarch64-unknown-linux-gnu ;;
    Linux:x86_64) kuru_target=x86_64-unknown-linux-gnu ;;
    *) fail 'unsupported platform; build from source with Rust' ;;
  esac
fi
case "$kuru_target" in
  aarch64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu) ;;
  x86_64-apple-darwin) fail 'Intel Macs (x86_64-apple-darwin) are no longer supported; v0.9.0 was the last release supporting them' ;;
  *) fail 'unsupported platform; build from source with Rust' ;;
esac

kuru_default_base=false
if [[ -z $kuru_base ]]; then
  kuru_default_base=true
  if [[ -n $kuru_version ]]; then
    kuru_base="https://github.com/replygirl/kuru/releases/download/v$kuru_version"
  else
    kuru_base=https://github.com/replygirl/kuru/releases/latest/download
  fi
else
  [[ -n $kuru_version ]] || fail 'a custom release base requires --version'
fi
case "$kuru_base" in
  https://*)
    kuru_authority=${kuru_base#https://}
    kuru_authority=${kuru_authority%%/*}
    [[ -n $kuru_authority && $kuru_authority != *@* &&
      $kuru_base != *'?'* && $kuru_base != *'#'* && $kuru_base != *\\* &&
      ! $kuru_base =~ [[:space:][:cntrl:]] ]] || fail 'release base must be HTTPS without credentials, whitespace, a query or fragment'
    command -v curl >/dev/null || fail 'curl is required for HTTPS downloads'
    ;;
  *://*) fail 'release base must be HTTPS or a local directory' ;;
  *) [[ -d $kuru_base ]] || fail 'release base must be HTTPS or an existing local directory' ;;
esac

if [[ -z $kuru_directory ]]; then
  [[ -n ${HOME:-} ]] || fail 'provide --install-dir when HOME is unset'
  kuru_directory="$HOME/.local/bin"
fi
[[ $kuru_directory != *$'\n'* && $kuru_directory != *$'\r'* ]] || fail 'install directory must not contain line breaks'
for kuru_tool in mkdir mktemp mkfifo head wc cat gzip tar chmod mv rm tr cmp sort cp; do
  command -v "$kuru_tool" >/dev/null || fail "$kuru_tool is required"
done
if command -v sha256sum >/dev/null; then
  kuru_hash_tool=sha256sum
elif command -v shasum >/dev/null; then
  kuru_hash_tool=shasum
else
  fail 'sha256sum or shasum is required'
fi

mkdir -p -- "$kuru_directory"
kuru_directory=$(cd -- "$kuru_directory" && pwd -P)
kuru_destination="$kuru_directory/kuru"
check_destination() {
  [[ ! -L $kuru_destination && ( ! -e $kuru_destination || -f $kuru_destination ) ]] || fail 'destination kuru must be a regular file, not a symlink or directory'
}
check_destination
kuru_stage=$(mktemp -d "$kuru_directory/.kuru-install.XXXXXX")

# Retain both process identities so signals and size limits stop the producer,
# including one that keeps its stream open after head has reached the cap.
bounded() {
  local kuru_limit=$1 kuru_output=$2 kuru_description=$3
  shift 3
  local kuru_pipe="$kuru_stage/stream" kuru_head_status kuru_source_status kuru_bytes
  mkfifo "$kuru_pipe"
  # Arm the reader first. On macOS, an immediately failing writer can otherwise
  # close during the FIFO open handoff and leave a later reader waiting forever.
  head -c "$((kuru_limit + 1))" < "$kuru_pipe" > "$kuru_output" &
  kuru_consumer=$!
  "$@" > "$kuru_pipe" &
  kuru_producer=$!
  if wait "$kuru_consumer"; then kuru_head_status=0; else kuru_head_status=$?; fi
  kuru_consumer=''
  (( kuru_head_status == 0 )) || fail "$kuru_description reader failed ($kuru_head_status)"
  kuru_bytes=$(wc -c < "$kuru_output")
  if (( kuru_bytes > kuru_limit )); then
    fail "$kuru_description exceeds size limit"
  fi
  if wait "$kuru_producer"; then kuru_source_status=0; else kuru_source_status=$?; fi
  kuru_producer=''
  rm -- "$kuru_pipe"
  (( kuru_head_status == 0 && kuru_source_status == 0 )) || fail "$kuru_description failed (producer $kuru_source_status, reader $kuru_head_status)"
}

fetch() {
  local kuru_name=$1 kuru_limit=$2 kuru_output=$3
  case "$kuru_base" in
    https://*)
      bounded "$kuru_limit" "$kuru_output" 'release download' \
        curl -q --fail --silent --show-error --location --globoff \
        --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 60 \
        "${kuru_base%/}/$kuru_name"
      ;;
    *) bounded "$kuru_limit" "$kuru_output" 'local release asset' cat -- "$kuru_base/$kuru_name" ;;
  esac
}

fetch SHA256SUMS "$kuru_manifest_limit" "$kuru_stage/SHA256SUMS"
# Bash strings cannot represent NUL. Reject it before interpreting text lines.
tr -d '\000' < "$kuru_stage/SHA256SUMS" > "$kuru_stage/manifest-text"
cmp -s "$kuru_stage/SHA256SUMS" "$kuru_stage/manifest-text" || fail 'checksum manifest contains NUL bytes'
kuru_manifest_pattern='^([[:xdigit:]]{64}) [ *]([^[:space:]]+)$'
kuru_matches=0
kuru_expected=''
kuru_name=''
while IFS= read -r kuru_line || [[ -n $kuru_line ]]; do
  kuru_line=${kuru_line%$'\r'}
  [[ -n $kuru_line ]] || continue
  [[ $kuru_line =~ $kuru_manifest_pattern ]] || fail 'malformed checksum manifest'
  kuru_hash=${BASH_REMATCH[1]}
  kuru_member=${BASH_REMATCH[2]}
  if [[ -n $kuru_version ]]; then
    [[ $kuru_member == "kuru-$kuru_version-$kuru_target.tar.gz" ]] || continue
  else
    [[ $kuru_member == kuru-*"-$kuru_target.tar.gz" ]] || continue
    kuru_selected=${kuru_member#kuru-}
    kuru_selected=${kuru_selected%"-$kuru_target.tar.gz"}
    [[ $kuru_selected =~ $kuru_version_pattern ]] || fail 'checksum manifest contains an invalid archive version'
  fi
  kuru_matches=$((kuru_matches + 1))
  kuru_expected=$kuru_hash
  kuru_name=$kuru_member
done < "$kuru_stage/manifest-text"
(( kuru_matches == 1 )) || fail 'checksum manifest must name the release archive exactly once'
if [[ -z $kuru_version ]]; then
  kuru_version=${kuru_name#kuru-}
  kuru_version=${kuru_version%"-$kuru_target.tar.gz"}
fi
if $kuru_default_base; then
  kuru_base="https://github.com/replygirl/kuru/releases/download/v$kuru_version"
fi
fetch "$kuru_name" "$kuru_archive_limit" "$kuru_stage/archive.tar.gz"
if [[ $kuru_hash_tool == sha256sum ]]; then
  kuru_actual=$(sha256sum < "$kuru_stage/archive.tar.gz")
else
  kuru_actual=$(shasum -a 256 < "$kuru_stage/archive.tar.gz")
fi
kuru_actual=${kuru_actual%% *}
kuru_expected=$(printf '%s' "$kuru_expected" | tr '[:upper:]' '[:lower:]')
[[ $kuru_actual == "$kuru_expected" ]] || fail 'release archive checksum mismatch; existing executable unchanged'

bounded "$kuru_archive_limit" "$kuru_stage/archive.tar" 'expanded release archive' gzip -dc "$kuru_stage/archive.tar.gz"
bounded "$kuru_manifest_limit" "$kuru_stage/names" 'archive inventory' tar -tf "$kuru_stage/archive.tar"
sort "$kuru_stage/names" > "$kuru_stage/sorted-names"
printf 'LICENSE\nREADME.md\nkuru\n' > "$kuru_stage/expected-names"
cmp -s "$kuru_stage/sorted-names" "$kuru_stage/expected-names" || fail 'release archive must contain exactly kuru, LICENSE and README.md without duplicates or other paths'
bounded "$kuru_manifest_limit" "$kuru_stage/types" 'archive inventory' tar -tvf "$kuru_stage/archive.tar"
kuru_entries=0
while IFS= read -r kuru_line || [[ -n $kuru_line ]]; do
  [[ ${kuru_line:0:1} == '-' ]] || fail 'release archive entries must be regular files, not links'
  if [[ $kuru_line == *' kuru' ]]; then
    [[ ${kuru_line:3:1} == x || ${kuru_line:3:1} == s ]] || fail 'release kuru entry is not executable'
  fi
  kuru_entries=$((kuru_entries + 1))
done < "$kuru_stage/types"
(( kuru_entries == 3 )) || fail 'release archive must contain exactly three regular files'
bounded "$kuru_archive_limit" "$kuru_stage/kuru" 'release executable' tar -xOf "$kuru_stage/archive.tar" -- kuru
[[ -s $kuru_stage/kuru ]] || fail 'release kuru entry is empty'
chmod 755 "$kuru_stage/kuru"

# The verified core README distinguishes historical three-member releases from
# releases that require a target-paired, checksum-verified support envelope.
bounded "$kuru_manifest_limit" "$kuru_stage/README.md" 'release README' tar -xOf "$kuru_stage/archive.tar" -- README.md
tr -d '\000' < "$kuru_stage/README.md" > "$kuru_stage/readme-text"
cmp -s "$kuru_stage/README.md" "$kuru_stage/readme-text" || fail 'release README contains NUL bytes'
kuru_marker_count=0
kuru_readme_line=0
while IFS= read -r kuru_line || [[ -n $kuru_line ]]; do
  kuru_readme_line=$((kuru_readme_line + 1))
  if [[ $kuru_line == *kuru-shell-support-format* ]]; then
    [[ $kuru_line == '<!-- kuru-shell-support-format: 1 -->' && $kuru_readme_line -le 64 ]] || fail 'release README has an invalid shell-support marker'
    kuru_marker_count=$((kuru_marker_count + 1))
  fi
done < "$kuru_stage/readme-text"
(( kuru_marker_count <= 1 )) || fail 'release README has duplicate shell-support markers'

if (( kuru_marker_count == 1 )); then
  kuru_support_name="kuru-$kuru_version-$kuru_target-shell-support.tar.gz"
  kuru_support_matches=0
  kuru_support_expected=''
  while IFS= read -r kuru_line || [[ -n $kuru_line ]]; do
    kuru_line=${kuru_line%$'\r'}
    [[ -n $kuru_line ]] || continue
    [[ $kuru_line =~ $kuru_manifest_pattern ]] || fail 'malformed checksum manifest'
    if [[ ${BASH_REMATCH[2]} == "$kuru_support_name" ]]; then
      kuru_support_matches=$((kuru_support_matches + 1))
      kuru_support_expected=${BASH_REMATCH[1]}
    fi
  done < "$kuru_stage/manifest-text"
  (( kuru_support_matches == 1 )) || fail 'checksum manifest must name the paired shell-support archive exactly once; existing executable unchanged'
  fetch "$kuru_support_name" "$kuru_support_limit" "$kuru_stage/support.tar.gz"
  if [[ $kuru_hash_tool == sha256sum ]]; then
    kuru_support_actual=$(sha256sum < "$kuru_stage/support.tar.gz")
  else
    kuru_support_actual=$(shasum -a 256 < "$kuru_stage/support.tar.gz")
  fi
  kuru_support_actual=${kuru_support_actual%% *}
  kuru_support_expected=$(printf '%s' "$kuru_support_expected" | tr '[:upper:]' '[:lower:]')
  [[ $kuru_support_actual == "$kuru_support_expected" ]] || fail 'shell-support archive checksum mismatch; existing executable unchanged'

  bounded "$kuru_support_limit" "$kuru_stage/support.tar" 'expanded shell-support archive' gzip -dc "$kuru_stage/support.tar.gz"
  bounded "$kuru_manifest_limit" "$kuru_stage/support-names" 'shell-support inventory' tar -tf "$kuru_stage/support.tar"
  sort "$kuru_stage/support-names" > "$kuru_stage/support-sorted-names"
  printf 'completions/_kuru\ncompletions/kuru.bash\ncompletions/kuru.fish\ncompletions/kuru.ps1\nman/kuru.1\n' > "$kuru_stage/support-expected-names"
  cmp -s "$kuru_stage/support-sorted-names" "$kuru_stage/support-expected-names" || fail 'shell-support archive must contain exactly five support files without duplicates or other paths'
  bounded "$kuru_manifest_limit" "$kuru_stage/support-types" 'shell-support inventory' tar -tvf "$kuru_stage/support.tar"
  kuru_entries=0
  while IFS= read -r kuru_line || [[ -n $kuru_line ]]; do
    [[ ${kuru_line:0:10} == '-rw-r--r--' ]] || fail 'shell-support archive entries must be regular files with mode 0644'
    kuru_entries=$((kuru_entries + 1))
  done < "$kuru_stage/support-types"
  (( kuru_entries == 5 )) || fail 'shell-support archive must contain exactly five regular files'
  mkdir -p -- "$kuru_stage/support/completions" "$kuru_stage/support/man"
  for kuru_member in completions/kuru.bash completions/_kuru completions/kuru.fish completions/kuru.ps1 man/kuru.1; do
    bounded "$kuru_support_member_limit" "$kuru_stage/support/$kuru_member" 'shell-support member' tar -xOf "$kuru_stage/support.tar" -- "$kuru_member"
    [[ -s $kuru_stage/support/$kuru_member ]] || fail 'shell-support member is empty'
    chmod 644 "$kuru_stage/support/$kuru_member"
  done

  # All support bytes are checked before publication of the executable. The
  # explicit install directory is the root for both its binary and support.
  kuru_share="$kuru_directory/share"
  kuru_support_parent="$kuru_share/kuru"
  for kuru_path in "$kuru_share" "$kuru_support_parent" "$kuru_support_parent/$kuru_version"; do
    [[ ! -L $kuru_path && ( ! -e $kuru_path || -d $kuru_path ) ]] || fail 'shell-support parent must be a real directory'
    mkdir -p -- "$kuru_path"
    [[ ! -L $kuru_path && -d $kuru_path ]] || fail 'shell-support parent changed identity'
  done
  kuru_support_destination="$kuru_support_parent/$kuru_version/$kuru_target"
  [[ ! -L $kuru_support_destination && ( ! -e $kuru_support_destination || -d $kuru_support_destination ) ]] || fail 'shell-support destination must be a real directory'
  mkdir -p -- "$kuru_support_destination"
  [[ ! -L $kuru_support_destination && -d $kuru_support_destination ]] || fail 'shell-support destination changed identity'
  for kuru_path in "$kuru_support_destination/completions" "$kuru_support_destination/man"; do
    [[ ! -L $kuru_path && ( ! -e $kuru_path || -d $kuru_path ) ]] || fail 'shell-support destination has unsafe directories'
    mkdir -p -- "$kuru_path"
    [[ ! -L $kuru_path && -d $kuru_path ]] || fail 'shell-support destination changed identity'
  done
  for kuru_member in completions/kuru.bash completions/_kuru completions/kuru.fish completions/kuru.ps1 man/kuru.1; do
    kuru_path="$kuru_support_destination/$kuru_member"
    [[ ! -L $kuru_path && ( ! -e $kuru_path || -f $kuru_path ) ]] || fail 'shell-support destination has an unsafe member'
    if [[ ! -e $kuru_path ]]; then
      cp -- "$kuru_stage/support/$kuru_member" "$kuru_path"
    fi
    cmp -s "$kuru_stage/support/$kuru_member" "$kuru_path" || fail 'shell-support destination differs from verified archive'
  done
  shopt -s nullglob dotglob
  kuru_children=("$kuru_support_destination"/*)
  (( ${#kuru_children[@]} == 2 )) || fail 'shell-support destination contains unexpected entries'
  kuru_children=("$kuru_support_destination/completions"/*)
  (( ${#kuru_children[@]} == 4 )) || fail 'shell-support destination contains unexpected completions'
  kuru_children=("$kuru_support_destination/man"/*)
  (( ${#kuru_children[@]} == 1 )) || fail 'shell-support destination contains unexpected man files'
  shopt -u nullglob dotglob
  for kuru_member in completions/kuru.bash completions/_kuru completions/kuru.fish completions/kuru.ps1 man/kuru.1; do
    [[ ! -L $kuru_support_destination/$kuru_member && -f $kuru_support_destination/$kuru_member ]] || fail 'shell-support destination is incomplete or unsafe'
    cmp -s "$kuru_stage/support/$kuru_member" "$kuru_support_destination/$kuru_member" || fail 'shell-support destination differs from verified archive'
  done
fi

check_destination
mv -f -- "$kuru_stage/kuru" "$kuru_destination"
if (( kuru_marker_count == 1 )); then
  kuru_man_parent="$kuru_share/man"
  kuru_man_directory="$kuru_man_parent/man1"
  for kuru_path in "$kuru_man_parent" "$kuru_man_directory"; do
    [[ ! -L $kuru_path && ( ! -e $kuru_path || -d $kuru_path ) ]] || fail 'partial install: stable man directory is unsafe; rerun this installer to repair'
    mkdir -p -- "$kuru_path"
    [[ ! -L $kuru_path && -d $kuru_path ]] || fail 'partial install: stable man directory changed identity; rerun this installer to repair'
  done
  kuru_man_destination="$kuru_man_directory/kuru.1"
  [[ ! -L $kuru_man_destination && ( ! -e $kuru_man_destination || -f $kuru_man_destination ) ]] || fail 'partial install: stable man destination is unsafe; rerun this installer to repair'
  cp -- "$kuru_support_destination/man/kuru.1" "$kuru_stage/kuru.1"
  mv -f -- "$kuru_stage/kuru.1" "$kuru_man_destination" || fail 'partial install: could not publish stable man file; rerun this installer to repair'
  cmp -s "$kuru_support_destination/man/kuru.1" "$kuru_man_destination" || fail 'partial install: stable man file differs; rerun this installer to repair'
fi
printf 'Installed Kuru %s at %s; add %s to PATH.\n' "$kuru_version" "$kuru_destination" "$kuru_directory"
