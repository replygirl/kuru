## Corrected generation boundary

## Integration contract

Communiqué 1.3.5 remains pinned. Its OpenAI provider posts to
`https://api.anthropic.com/v1/chat/completions`, supports tool calls and reads
Bearer credentials from OPENAI_API_KEY. Anthropic's official compatibility API
omits thinking blocks from its response, so Claude Sonnet 5 can retain adaptive
thinking without the native parser failure. Only the notes step maps the existing
ANTHROPIC_API_KEY_COMMUNIQUE secret to that child environment name. No OpenAI
credential, additional secret, relay or app-provider change is introduced.

The compatibility endpoint is a temporary integration choice until upstream's
native Anthropic parser supports Claude 5 response blocks. Use no sampling
overrides, no response-shape rewriting and no fallback to a weaker model.
Preserve bounded subprocess execution, failure cleanup and immutable source/tag
checks. Errors must not include credential values or raw provider payloads.

Current product documentation, read with git show from the selected commit,
supplements the first-release root inventory and the tool's historical log.
The source snapshot is authoritative for present behavior; older commits explain
changes, not the current defaults. Keep product facts in context and writing
instructions in system_extra. Output limits (450 words, at most ten bullets)
are checked deterministically; they do not establish factual accuracy.

## Acceptance and limits

Test the real pinned Communiqué binary against a local compatible API fixture,
including a repository-file tool call and final notes submission, correct model,
endpoint, authentication and unsupported/error shapes. A regression must prove
the former native route fails on Claude 5 thinking while the compatible route
handles the supported response. A second regression must reject the actual
overlong-bullet failure shape without producing output, while preserving an
existing reviewed output. Verify source snapshots exclude uncommitted edits.
Reject nonignored untracked files too: Communiqué's repository search can read
them even though its file-read tool restricts itself to tracked files. Ignored
build products may remain. Check an existing output before this clean-tree gate
so a reviewed artifact retains its explicit diagnostic and bytes.

Review actual next-run notes against the selected source before publication;
record claims and source evidence separately from fixture success. A green
API fixture proves protocol behavior, not model reliability. No deployment is
claimed until the same Release run publishes and completes inline Pages.

## Operational surface

Generation remains in the existing Ubuntu notes job, using the selected version
commit in parallel with final validation. The publisher still requires notes and
all native builds, which depend on validation. Anthropic's official HTTPS API
is the model endpoint; the job also reads GitHub for release context. There is
no added listener or server in production. Local fixtures bind loopback on an ephemeral port and terminate
with their test. The existing scoped secret remains the sole provider credential.
Requests stay within Communiqué's bounded tool loop and the wrapper's ten-minute
timeout. The four native archives, publisher permissions and Pages stages are
unchanged.

Sources: Communiqué v1.3.5 src/providers/openai.rs and config.rs;
https://platform.claude.com/docs/en/cli-sdks-libraries/libraries/openai-sdk;
https://platform.claude.com/docs/en/models/sonnet-5/whats-new-sonnet-5.
