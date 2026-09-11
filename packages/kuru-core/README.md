# kuru-core

Shared framework profiles, extensible provider messages and layered configuration.
Each built-in member is a capable peer with complementary
working tendencies. Frameworks are computational interpretations of historical
psychological models, without clinical or sentience claims. The IFS profile
includes multiple managers, firefighters, and exiles. Jungian collective memory
is scoped to the project in v1.

Part UUIDs are deterministic from versioned mode, role, and name seeds.
Relationship UUIDs include the relationship kind and a sorted, length-prefixed
membership list. The same unordered dyad, triad, or tetrad retains its identity;
messages describe the direction of protection. The runtime checks membership
existence and enforces namespace access. This crate does not infer authorization
from a caller knowing another part's namespace.

Configuration overlays defaults with an explicit user file, ancestor
`.kuru/config.toml` files from outermost to most local, and an explicit local file.
Tables merge recursively; scalar values and arrays replace. Unknown keys and
types fail at each layer. Semantic validation follows the final merge. Explicit
user/local paths must exist; absent ancestor files are skipped. Each file is
limited to 256 KiB, and combined content to 1 MiB. Instructions follow the same
ancestor order with explicit local precedence. Provider effort names remain
strings so new model settings need no schema change. `dream_every = 0` disables
periodic dreaming. MCP endpoints specify either a process command or HTTP(S)
URL; when switching transports across layers, remove the incompatible earlier
configuration, since TOML has no null-value deletion syntax.

Storage lives in `packages/kuru-memory`: full Dolt owns revisioned histories,
candidate branches, atomic state updates and preserved legacy SQLite import.
Core retains only memory configuration and shared identity contracts. Built-in
identity seed changes require explicit migration because IDs address durable
memories. Never silently reset or recreate existing data to resolve an unknown
schema; preserve recoverable backups and test upgrades with persisted fixtures.
See [memory storage](../../docs/memory.md) for the storage and recovery contract.
