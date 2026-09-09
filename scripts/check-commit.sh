#!/usr/bin/env bash
set -euo pipefail
IFS= read -r kuru_title < "$1" || true
if [[ ! "$kuru_title" =~ ^(feat|fix|refactor|perf|build|ci|docs|test|chore|style|revert)(\([a-zA-Z0-9._/-]+\))?\!?:\ .+ ]]; then
  echo "Use a conventional commit title, such as feat(runtime): add peer mailboxes." >&2
  exit 1
fi
