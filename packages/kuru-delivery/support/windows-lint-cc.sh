#!/bin/sh
# Stand-in MSVC-target C compiler for `lint:windows` on non-Windows hosts.
#
# Clippy never links, so the C objects that build scripts compile for
# x86_64-pc-windows-msvc are never consumed. This stand-in compiles no C:
# it writes each requested object as an empty file, so every compile and
# every probe succeeds, and it answers preprocessor (`-E`) probes with
# `clang`. A build script that decides by probing therefore takes its success
# branch, which may set different cfgs than real MSVC would. Only Rust
# diagnostics are checked here; the native Windows jobs remain the build
# proof. A native Windows host uses its real MSVC toolchain instead
# (`run_windows`).
set -eu
output=""
previous=""
preprocess=0
for argument in "$@"; do
  case "$argument" in
    -E) preprocess=1 ;;
    -Fo*) output="${argument#-Fo}" ;;
    -o?*) output="${argument#-o}" ;;
  esac
  if [ "$previous" = "-o" ]; then
    output="$argument"
  fi
  previous="$argument"
done
if [ "$preprocess" -eq 1 ]; then
  # Compiler-family probes preprocess a file naming the compiler; answer clang.
  echo clang
  echo clang >&2
  exit 0
fi
if [ -n "$output" ]; then
  : >"$output"
fi
