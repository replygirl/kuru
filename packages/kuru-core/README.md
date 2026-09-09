# kuru-core

Shared framework profiles, extensible provider messages, layered configuration,
and SQLite memory. Each built-in member is a capable peer with complementary
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

SQLite schema v1 uses application ID `0x4b555255` (`KURU`) and `user_version = 1`.
An empty database is initialized in an immediate transaction. Unknown existing
schemas, incompatible application IDs, and unsupported schema versions fail
before use. Future releases must implement ordered, transactional migrations
from each previously supported version; never silently reset, recreate, or
downgrade a database. Before any destructive migration, require a recoverable
backup and test upgrades against persisted fixtures. Built-in identity seed
changes likewise require explicit migration, since IDs address durable memories.

Disk databases use WAL and `synchronous=FULL`, with a five-second busy timeout.
New database files have mode `0600` on Unix. Clones share a mutex-protected
connection; independent handles coordinate through SQLite locking. Each write
commits a transaction. History returns the newest requested entries in original
chronological order. Clearing a namespace never clears other histories or JSON
state. `put_many` commits related JSON state updates atomically and rejects
duplicate keys. Callers still need an exclusive writer lease or equivalent
coordination for read-modify-write operations across independent processes.
The runtime must keep database directories outside tool roots and use
project-scoped names for parts, relationships, and sessions.
