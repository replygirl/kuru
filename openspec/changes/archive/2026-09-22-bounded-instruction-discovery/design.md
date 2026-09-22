## Context

`ConfigSnapshot` currently reads one `AGENTS.md` in each ancestor directory, binds ordered file identities and exact bytes into one prompt-authority claim, and gives the runtime the captured projection after workspace preflight. The first P01 slice established caller-local and managed configuration without changing that instruction path. This change extends the same immutable capture; nested subtree activation is the dependent follow-on and is deliberately absent from this change's runtime API.

## Goals / Non-Goals

**Goals:** Preserve the existing exact-root workspace review while composing the two common instruction filenames and explicit imports, including this repository's `CLAUDE.md` → `@AGENTS.md` convention. Keep a useful project launch when an instruction source exceeds the byte or graph cap, with a truthful notice before inference.

**Non-Goals:** Dynamically activate nested instructions, scan subtrees speculatively, infer imports from prose, alter tool permissions, interpret Markdown precedence semantically, or treat an explicit caller config as approval of repository instructions.

## Decisions

1. **Source order and duplicate rule.** Visit ancestor directories from outermost to project root. In each directory, visit `AGENTS.md` and then `CLAUDE.md`; an import expands at its directive position before the containing file resumes. Each checked physical file identity is rendered at most once in that snapshot. Thus a sibling `CLAUDE.md` importing its already visited `AGENTS.md` removes that import directive from the rendered text without injecting a second copy. Later applicable local material retains precedence; no filename alone grants caller authority. Rejected alternative: concatenating both files before import expansion, which duplicates this repository's canonical wrapper.

2. **Explicit, confined import syntax.** A standalone line whose trimmed content begins with `@` and names a relative `.md` path is an import directive; other text remains literal. Resolve it relative to the containing file, lexically normalize `.`/`..`, and require it to remain under the directory of the original top-level `AGENTS.md` or `CLAUDE.md` source. Open every directory component and final regular file through the checked platform directory API, which rejects links and unstable names; do not follow an imported symlink or read outside that source scope. A missing, invalid or escaping import fails capture with a redacted source-category diagnostic; only specified capacity exhaustion is an omission. Rejected alternative: `canonicalize()` followed by a normal file open, which permits a path to be replaced between checks and can read outside the reviewed scope.

3. **Bound the graph and keep usable material.** Retain the existing 256 KiB per-file and 1 MiB combined content caps; add at most 128 checked source paths and eight import edges of depth. A source that exceeds a byte cap is wholly omitted, not truncated. A branch that exceeds the graph cap is not traversed; a cycle is detected by the active normalized path or checked file-identity stack and its closing edge is omitted. Every such omission increments a count and contributes a bounded escaped notice to the prompt projection and a launch diagnostic. The same source may be imported again only as a no-op duplicate, without adding separator text. The source claim binds only actual active sources, their exact bytes, path digest, file identity and containing directory identity in rendered encounter order. Rejected alternative: hard-failing the whole project on one oversized file or silently clipping its text.

4. **Capture once, review once.** `ConfigSnapshot` owns the complete rendered text and bounded omission report. Its manifest is derived from the captured active sources before memory/provider/tool side effects. The CLI displays any omission report before the first applicable dispatch; trust status displays it during inspection. A later file edit or replacement cannot alter this snapshot's prompt, and a new launch derives a changed manifest. The existing once/persistent workspace approval mechanism applies to imported sources exactly as it applies to `AGENTS.md`; no new approval store or user-local classification is introduced.

## Risks / Trade-offs

- [A root instruction in a broad ancestor may import more files than expected] → Require a relative `.md` path confined to that top-level source directory, checked regular-file opens, a small graph cap, and exact manifest review.
- [A skipped import can change intended prose order] → Preserve each import's directive position for included content and put an explicit omission notice at the skipped position; never pass a raw unresolved directive as if it had been applied.
- [A changed or replaced source could differ after review] → Use only captured bytes during the invocation; bind path, directory and file identity plus exact content to the manifest so the next snapshot requires review if authority changes.

## Operational surface

This change runs inside the existing local `kuru` process and its current CLI/TUI workspace review. It adds no bind address, container or runner, required secret, connection, binary architecture, or runtime dependency. Import scanning is local and bounded by the file/graph caps above; no network or subprocess is part of instruction capture.
