# Initial implementation contract

Intent: Kuru is a Rust chat harness whose native unit of cognition is a pool of
equal persistent parts. No LLM supervisor owns other actors. Frameworks define
roles and tendencies; each part can message any peer, report modeled state,
propose a relationship, and participate in dreaming. Temporary relationships of
2–4 members can become the speaking identity and own separate durable memory.
This is a computational interpretation of psychological frameworks, not a
clinical treatment or a claim of consciousness.

Ownership during initial development:
- core agent: `packages/kuru-core/**` (config, framework, SQLite memory, shared types).
- connectors agent: `packages/kuru-connectors/**` (providers, JSON-RPC, MCP, tools).
- workflow agent: root tooling/config, scripts, CI/release, docs, cospec artifacts.
- primary agent: `packages/kuru-runtime/**`, `apps/kuru-tui/**`, integration and final QA.

All crates use workspace dependencies. Changes to public interfaces must be
communicated before consumers are changed. No independent commits or external
publication. Do not read/print credential files. Test using deterministic fake
providers/protocol peers; optionally smoke test installed Codex without printing
tokens. Aim for >=90% meaningful line coverage across the whole workspace.

## Shared Rust interfaces (`kuru_core`, root re-exports)

All data types derive Debug, Clone, Serialize, Deserialize unless inappropriate.
Use anyhow::Result and serde_json::Value. IDs are strings, generated with UUID.

`Mode`: Ifs, Polyvagal, Freudian, Jungian; Display/FromStr with lowercase names.
`Part { id: String, name: String, role: String, instruction: String, active: bool }`
`Framework { mode: Mode, parts: Vec<Part> }`, `Framework::builtin(mode: Mode)`.
`RelationshipKind`: Protection, Polarization, Alliance; Display/FromStr.
`Relationship { id: String, kind: RelationshipKind, members: Vec<String> }`;
`Relationship::new(kind, members) -> Result<Self>`: 2–4 distinct members,
canonical ID independent of input order. Membership existence checked by runtime.
`Message { role: String, content: String }`
`ToolSpec { name: String, description: String, parameters: Value }`
`ToolCall { id: String, name: String, arguments: Value }`
`ModelInfo { id: String, name: String, efforts: Vec<String>, default_effort: Option<String> }`
`CompletionRequest { actor: String, instructions: String, messages: Vec<Message>, model: String, effort: Option<String>, tools: Vec<ToolSpec> }`
`Completion { text: String, calls: Vec<ToolCall>, input_tokens: u64, output_tokens: u64 }`

`Config` fields: mode: Mode, provider: String (codex/responses/demo), model: String
(default auto), effort: Option<String>, max_rounds: usize (3), max_tool_calls:
usize (12), max_parallel: usize (4), dream_every: usize (8), dream_on_exit: bool,
max_parts: usize (16), allow_shell: bool (false), allow_write: bool (false),
codex_command: String (codex), api_base: String (https://api.openai.com/v1),
api_key_env: String (OPENAI_API_KEY), mcp: BTreeMap<String, McpConfig>,
external_agents: BTreeMap<String, String> (alias -> A2A URL).
`McpConfig { command: Option<String>, args: Vec<String>, url: Option<String>, env: BTreeMap<String,String> }`.
`Config::default()`, `Config::load(user: Option<&Path>, project: &Path, local: Option<&Path>) -> Result<Self>`;
user file then ancestor `.kuru/config.toml` outer-to-inner then optional local
file. Unknown config keys fail clearly. Paths are explicit; CLI resolves user/data.
`Config::validate() -> Result<()>`.
`load_instructions(project: &Path) -> Result<String>` reads ancestor AGENTS.md
with size limits and most-local precedence described in combined text.

`MemoryStore::open(path: &Path) -> Result<Self>`, cloneable thread-safe handle.
`MemoryStore::in_memory() -> Result<Self>`.
Methods use `&self`: `append(namespace: &str, role: &str, content: &str) -> Result<()>`,
`history(namespace: &str, limit: usize) -> Result<Vec<Message>>`,
`put(key: &str, value: &Value) -> Result<()>`, `get(key: &str) -> Result<Option<Value>>`,
`clear(namespace: &str) -> Result<()>`. SQLite durable transactional writes.
Runtime owns namespace access policy. State stores are outside workspace and
never supplied as tool roots. Project namespace uses hash of canonical path.

## Connector interfaces (`kuru_connectors`, root re-exports)

`#[async_trait] trait Provider: Send + Sync { async fn models(&self) -> Result<Vec<ModelInfo>>; async fn complete(&self, request: CompletionRequest) -> Result<Completion>; }`
`provider(config: &Config, cwd: &Path) -> Result<Arc<dyn Provider>>` (async factory
if required: decide immediately and notify primary).
Codex adapter uses the supported app-server and dynamic tools. It is an
inference/auth transport ONLY: each call uses an ephemeral read-only thread,
built-in shell/agent spawning disabled where supported. Kuru owns memories and
tool execution. Dynamic calls returned to runtime (respond with receipt then
interrupt/finish turn as appropriate); no hidden privileged execution.
Alternative acceptable implementation: structured JSON output schema containing
text and calls, with built-in tools disabled, parsed into Completion. Verify real
app-server compatibility. Auth uses Codex's login/status/logout subprocesses;
never implement unofficial OAuth or copy token stores. Responses provider uses
standard function tools and includes model reasoning effort. Models discovered
dynamically, preserve unknown effort strings for future models.

`ToolHost::new(root: &Path, config: &Config) -> Result<Self>` (async if required;
communicate). `async fn specs(&self) -> Result<Vec<ToolSpec>>`,
`async fn execute(&self, name: &str, args: Value) -> Result<String>`.
Built-in tool names: file_read, file_write, file_delete, file_list, shell.
Enforce root/symlink containment for file tools, protected instruction/config and
memory paths, read/write opt-in, shell opt-in + timeout + output cap. Shell
permission explicitly means process authority (do not claim cwd is a sandbox).
MCP stdio and Streamable HTTP initialize/list/call with timeouts, limits and
proper shutdown. Namespace MCP tools to prevent built-in collisions.

`async fn a2a_send(url: &str, message: &str, context_id: &str) -> Result<String>`
implements A2A 1.0 SendMessage JSON-RPC (confirm current official wire schema).
Primary owns the internal peer bus and A2A server. Connector agent should share
confirmed wire shapes so client and server agree.

## Runtime/TUI intent

Tokio actor mailboxes, bounded total peer rounds/tool calls/concurrency, every
peer can address every peer. Idle actors retain isolated memory. User input
activates pool, peer decisions select next recipients and speaking relationship.
Shared user transcript is separate from private part history; no accidental
all-part prompt concatenation. Dreaming collects isolated proposals, validates
and applies membership updates, archives retired parts, retains at least one
part per role and keeps the user able to inspect/reverse changes. Relationship
history accessible only to participating group. Jungian collective namespace
can persist across projects only when explicitly configured in a future option;
v1 project persistence is sufficient and must be documented.

TUI: chat, multiline entry, model/effort/mode commands or selectors, status,
parts/relationships panel, cancel, session persistence/resume, dream command.
CLI: default TUI, run prompt (--json), login/logout/auth status, models,
config, sessions, dream, update. Demo provider makes install verifiable without
credentials. Tests must distinguish demo operation from real provider coverage.
