#!/usr/bin/env python3
"""Check repository metadata invariants that formatters cannot enforce."""

import sys
from pathlib import Path

import tomllib

root = Path(__file__).resolve().parent.parent
errors = []
manifest = tomllib.loads((root / "Cargo.toml").read_text())
config = tomllib.loads((root / "mise.toml").read_text())
channel = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"][
    "channel"
]
if config["tools"]["rust"] != channel:
    errors.append("Rust pins differ between mise.toml and rust-toolchain.toml")
for name, dependency in manifest["workspace"]["dependencies"].items():
    version = dependency if isinstance(dependency, str) else dependency.get("version")
    if version is not None and not version.startswith("="):
        errors.append(f"workspace dependency {name} must be exactly pinned")
if not (root / "AGENTS.md").read_text().startswith("# Kuru\n"):
    errors.append("AGENTS.md must contain the canonical repository instructions")
if (root / "CLAUDE.md").read_text().strip() != "@AGENTS.md":
    errors.append("CLAUDE.md must import the canonical instructions with @AGENTS.md")
for directory in manifest["workspace"]["members"]:
    if not (root / directory / "Cargo.toml").is_file():
        errors.append(f"workspace member missing: {directory}")
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
print("Repository metadata invariants passed.")
