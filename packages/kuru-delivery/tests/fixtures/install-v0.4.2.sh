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
    Darwin:x86_64) kuru_target=x86_64-apple-darwin ;;
    Linux:aarch64|Linux:arm64) kuru_target=aarch64-unknown-linux-gnu ;;
    Linux:x86_64) kuru_target=x86_64-unknown-linux-gnu ;;
    *) fail 'unsupported platform; build from source with Rust' ;;
  esac
fi
case "$kuru_target" in
  aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu) ;;
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
for kuru_tool in mkdir mktemp mkfifo head wc cat gzip tar chmod mv rm tr cmp sort; do
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
  "$@" > "$kuru_pipe" &
  kuru_producer=$!
  head -c "$((kuru_limit + 1))" < "$kuru_pipe" > "$kuru_output" &
  kuru_consumer=$!
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
check_destination
mv -f -- "$kuru_stage/kuru" "$kuru_destination"
printf 'Installed Kuru %s at %s; add %s to PATH.\n' "$kuru_version" "$kuru_destination" "$kuru_directory"
