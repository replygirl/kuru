## Context

`ToolHost::execute` currently returns `Result<String>` directly from file, shell
and MCP implementations. File reads are text; listings, shell output and MCP
responses are known JSON at their producers. Runtime errors are formatted and
then persisted as tool messages, so filtering only a CLI renderer would leave a
durable and provider-facing bypass.

The tool producers have different existing bounds. A file read is at most 2 MiB,
a listing is capped at 10,000 entries, shell stdout and stderr are each capped at
2 MiB, and MCP messages/responses are independently bounded. Actual stdio MCP
stderr capture does not yet have the owned RPC/drain boundary required to publish
a safe excerpt.

## Goals / Non-Goals

**Goals:**

- Make one connector boundary authoritative for successful and failed tool
  output while retaining the public `Result<String>` API.
- Preserve semantic JSON structure, unmatched text and each producer's current
  limits.
- Make the finite scanner usable whole-value and incrementally with identical
  chunk/EOF behavior.

**Non-Goals:**

- Detect arbitrary/high-entropy/encoded/transformed secrets or validate provider
  credentials.
- Inspect credential stores or enumerate environment values.
- Capture MCP stderr, redesign Rpc/process ownership, alter MCP availability, or
  retrospectively rewrite memory.
- Change tool inputs, side effects, configuration, public JSON fields or
  dependencies.

## Decisions

### One private typed projection before the public boundary

Each producer returns private `Text`, `Json`, or application-error content.
`ToolHost::execute` projects and serializes it once. Known JSON is never confused
with arbitrary text which merely parses as JSON. A safe outward error stores only
a typed category and already-projected detail; it does not retain the raw error as
a source. MCP status logic consumes the typed category before formatting.

Filtering separately in the CLI/runtime was rejected because it would duplicate
policy and allow persisted or provider-facing raw output. Changing the public
return type was rejected because no new public result shape is needed.

### Finite ASCII detector table

Use the stable marker `[REDACTED:recognized-secret]`, merge overlapping/adjacent
matches, and preserve every unmatched byte. The initial table is:

| Detector | Recognition |
| --- | --- |
| Authorization | Bare or matching-quoted ASCII `Authorization`/`Proxy-Authorization`, optional horizontal whitespace, colon, optional quoted value, Basic/Bearer scheme and nonempty RFC-alphabet credential; replace credential only |
| OpenAI | Longest-first `sk-svcacct-`, `sk-proj-`, `sk-` plus at least 16 `[A-Za-z0-9_-]` bytes and a token boundary |
| GitHub | Official `ghp_`, `github_pat_`, `gho_`, `ghu_`, `ghs_`, `ghr_` prefix plus at least 8 `[A-Za-z0-9_.-]` bytes and a token boundary |
| AWS | `AKIA`/`ASIA` plus exactly 16 uppercase alphanumeric bytes and a token boundary |
| Private key | Exact five-hyphen begin/end delimiters for PRIVATE KEY, ENCRYPTED PRIVATE KEY, RSA PRIVATE KEY, EC PRIVATE KEY or OPENSSH PRIVATE KEY |
| Context | Bare or matching-quoted exact ASCII case-insensitive sensitive name, optional horizontal whitespace, `=`/`:`, optional whitespace and nonempty quoted or delimited value |

Sensitive names are `openai_api_key`, `api_key`, `apikey`, the three standard AWS
credential names, `github_token`, `gh_token`, `access_token`, `refresh_token`,
`auth_token`, `client_secret`, `password`, `passwd`, and `private_key`.

The OpenAI/GitHub minimums and AWS shape are local false-positive controls, not
provider validity. Do not add generic entropy, base64, JWT or hex scanning. Format
bases are the official OpenAI API-key examples/reference, GitHub authentication
and secret-scanning references, AWS access-key documentation, and RFC 7468, RFC
6750 and RFC 7617.

### Sensitive JSON fields select their values

Recursively scan every string key and string value. When an original key is an
exact contextual sensitive name or exact `Authorization`/`Proxy-Authorization`
under ASCII case folding, replace its complete associated value with the marker,
including non-string values. For other keys, replace only recognized spans and
preserve surrounding key text. Reject the whole projection with a fixed safe
error if projected keys collide rather than dropping a member. Preserve siblings
and enclosing structure.

Treating keys and values as unrelated strings was rejected because
`{"api_key":"opaque"}` and `{"Authorization":"Bearer opaque"}` would leak.
Parsing arbitrary text as JSON was rejected because file bytes and formatting
would change.

### Bounded streaming state and producer-relative growth

The scanner retains only the longest fixed prefix plus quote delimiters. After a
context/header name is recognized, it emits name and arbitrary whitespace while
remembering only a small phase; once nonempty value content begins it emits the
marker and suppresses the value. Private-key indentation is emitted immediately,
then a fixed delimiter automaton suppresses the body. AWS retains its fixed
20-byte candidate until a following boundary or EOF proves the match. EOF flushes
an unrecognized candidate but never flushes a recognized unfinished value/block.

Run in linear time against a fixed rule table. Bound projected text or serialized
JSON to checked `6 * original_bytes + 256`, above the grammar's maximum marker
expansion, and return a fixed withheld error on an impossible breach. This is a
bound on output bytes and requested output-buffer capacity. Grow buffers
geometrically within that bound so repeated small writes have amortized linear
copying cost. Count original JSON serialization without retaining a second raw
serialized copy, and check projected serialization before each append. JSON
value/container storage and allocator rounding are separate linear overhead;
the formula is not a total-process heap limit or a new input/combined-output
cap. Redaction precedes truncation and human control escaping.

A regex/entropy framework and a global 2 MiB output cap were rejected because
they would add dependencies, false positives and a shell/list compatibility
regression.

### Stderr capability is prepared, not activated

Test the scanner with adversarial synthetic byte chunks and EOF now. A later
owned stdio RPC/drain change may feed bytes through it before retaining a bounded
human-only stderr tail. This feature neither captures nor publishes MCP stderr
and makes no RPC cleanup claim.

### Runtime budgets preserve whole replacement markers

Projection runs once in connectors. Its pure exported
`truncate_tool_output(text, max_bytes) -> String` helper handles already-projected
tool text without scanning for credentials again. Runtime uses it at
`engine::tool_result`, `actor::bounded_receipt`, and the optional-history
truncation branch only when the message role is `tool`. Generic chat truncation,
tool call IDs, existing byte caps, JSON serialization, and metadata-only activity
remain unchanged. Legacy tool history remains text at the optional-history
boundary; this does not add parsing requirements to old stored messages.

The helper retains existing `[truncated]`/small-budget ellipsis behavior. If a
cut crosses the exact redaction marker and the complete marker plus truncation
suffix fits the current budget, shorten the preceding UTF-8 prefix to retain
the marker whole. That shortened prefix must not bisect an earlier marker. If
the budget cannot fit a complete marker, omit that marker wholly and keep the
bounded truncation indication. No projection marker may be introduced as a
partial token by truncation. This is not a promise to retain every marker or
every output byte when a context budget deliberately omits content, and it is
not the later head/tail feature.

Test every marker cut position, adjacent markers, multibyte text, small budgets,
JSON expansion and both current/optional receipt limits. Real runtime evidence
must show complete markers in retained tool content while source data and prior
durable history remain unchanged.

## Operational surface

The interactive surface is the existing `kuru tool` stdout/stderr and the
existing runtime/TUI rendering of persisted tool messages. This change adds no
listener, bind address, container boundary, secret input, connection limit,
binary, architecture or deployment topology. The connector applies the same
projection before either caller receives the existing `Result<String>`.

## Integration contract

Built-in producers identify their own result as text or JSON. Stdio and HTTP MCP
adapters hand validated `serde_json::Value` application results and typed
transport/protocol failures to the same projection; no SDK or wire schema changes.
Fixtures use isolated local stdio children and HTTP listeners with synthetic
credentials. MCP alias/route authority and external IDs remain owned by the
existing connector code, and projected display strings never become route or
status authority.

## Risks / Trade-offs

- [Known secret shape is missed] → Publish the exact finite detector set and
  false-negative boundary; add formats only through reviewed rule changes.
- [Examples or documentation are redacted] → Keep contextual/provider floors
  explicit and test representative false-positive controls.
- [JSON key projection collides] → Return one fixed safe withheld-result error;
  never overwrite a member.
- [Marker expansion grows output] → Enforce the checked producer-relative bound
  without narrowing existing producer admission.
- [Raw errors bypass projection] → Construct outward errors from typed category
  plus projected detail and test all formatting/source-chain modes.
