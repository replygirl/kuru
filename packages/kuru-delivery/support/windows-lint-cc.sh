#!/bin/sh
# Stand-in MSVC-target C compiler for `lint:windows` on non-Windows hosts.
#
# Clippy never links, so the C objects that build scripts compile for
# x86_64-pc-windows-msvc are never consumed. This stand-in compiles nothing:
# it writes each requested object as an empty file and succeeds, so build
# scripts such as aws-lc-sys, ring and libsqlite3-sys finish and Clippy can
# check the Rust code gated on `cfg(windows)`. A native Windows host uses its
# real MSVC toolchain instead (`run_windows`). A build script that needs real
# compiler output fails loudly here; it cannot make the lint pass falsely.
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
