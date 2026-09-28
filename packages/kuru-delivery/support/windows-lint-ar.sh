#!/bin/sh
# Stand-in MSVC-target archiver for `lint:windows` on non-Windows hosts; see
# windows-lint-cc.sh. It writes each requested library as an empty file.
set -eu
for argument in "$@"; do
  case "$argument" in
    -out:* | -OUT:* | /out:* | /OUT:*) : >"${argument#*:}" ;;
    *.lib | *.a) [ -e "$argument" ] || : >"$argument" ;;
  esac
done
