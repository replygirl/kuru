#!/usr/bin/env bash
set -euo pipefail

binary=${KURU_SHELL_SUPPORT_BINARY:?select an already-built release binary}
output=${KURU_SHELL_SUPPORT_OUTPUT:?select a new private output directory}
[[ $binary == /* && -f $binary && ! -L $binary ]] || {
  printf 'shell support binary must be an absolute regular file\n' >&2
  exit 1
}
[[ $output == /* && ! -e $output && ! -L $output && -d ${output%/*} ]] || {
  printf 'shell support output must be a new directory under an existing parent\n' >&2
  exit 1
}

mkdir -- "$output"
mkdir -- "$output/completions" "$output/man"
"$binary" completions bash > "$output/completions/kuru.bash"
"$binary" completions zsh > "$output/completions/_kuru"
"$binary" completions fish > "$output/completions/kuru.fish"
"$binary" completions powershell > "$output/completions/kuru.ps1"
"$binary" man > "$output/man/kuru.1"

for file in "$output"/completions/* "$output"/man/kuru.1; do
  [[ -s $file ]] || { printf 'generated shell support is empty\n' >&2; exit 1; }
  bytes=$(wc -c < "$file")
  (( bytes <= 512 * 1024 )) || { printf 'generated shell support exceeds bound\n' >&2; exit 1; }
done
