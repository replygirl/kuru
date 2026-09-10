## Context

The notes adapter enforces source identity and bounded output before persisting
Markdown. It also rejects word/bullet counts, which are editorial preferences
introduced by the previous repair and absent from the durable release contract.

## Decisions

Keep one Communiqué invocation and preserve its complete bounded regular UTF-8
output when nonempty. Do not rewrite, truncate or silently summarize the draft.
Keep concision guidance in communique.toml; a saved artifact enables actual prose
review independently of punctuation and list layout. Existing output is still
protected by atomic no-clobber persistence.

The 100 KB file bound, clean exact-SHA checkout, source context limit, version/tag
checks, provider-failure privacy and 600-second timeout remain enforced. No extra
credentials or model retry loop is added. Publication and inline Pages sequencing
remain unchanged.

## Operational surface

The existing Ubuntu notes job invokes the pinned Communiqué binary with the
existing scoped Anthropic secret through its configured compatibility adapter.
No ports, runners, secrets, workflow inputs or native build targets change.

## Integration contract

Communiqué writes a Markdown file after its tool-call generation loop. The Rust
adapter validates and atomically persists that file for the existing artifact
upload step. Real-binary HTTP fixtures retain the existing model, authentication
and tool-result replay contract while changing only the draft-length expectation.

## Verification boundary

Real pinned Communiqué fixtures must show formerly rejected 17-bullet and
451-word drafts preserved exactly, with a failing-before/passing-after run.
Existing invalid-output/source/overwrite regressions must keep passing. Review
the next actual live generated artifact against its selected source before
publication; fixture success does not establish factuality.
