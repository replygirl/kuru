# Contributing to Kuru

Kuru follows [aligned-team/cospec](https://github.com/aligned-team/cospec) as its
repository and change-workflow reference. The repository is private for now;
public availability is planned. Contributions use the same process in either case.

## Setup

```sh
gh repo clone replygirl/kuru
cd kuru
mise trust
mise install
mise run setup
```

The monorepo uses `apps/` and `packages/`, centrally pinned tools and dependencies,
and hk hooks. Run commands through mise. See [development](docs/development.md)
for the full task catalog and the meaningful 90% workspace coverage requirement.

[AGENTS.md](AGENTS.md) is the shared instruction source for coding assistants;
Claude Code imports it through `CLAUDE.md`. Cospec workflows are generated for
Claude Code, Codex and OpenCode. See [agent setup](docs/development.md#coding-assistants)
for their locations and regeneration commands.

## Changes and review

Create a branch from `main`. Start substantive work with a typed cospec change:

```sh
git switch -c feat/my-change
mise run cospec -- new feat my-change
mise run cospec -- instructions proposal --change my-change
# Author the required artifacts and acceptance evidence.
mise run cospec -- validate my-change --strict
mise run cospec -- apply my-change
# Implement and verify the observable behavior.
mise run check
mise run cospec -- archive my-change
```

Archive through cospec before the final branch commit and before merging. Keep
application behavior, documentation, dependencies and lockfiles consistent. Fix
failed checks rather than bypassing hooks. Use conventional commit and PR titles,
for example `fix: restore project mode after restart`.

Open a pull request to `main` with the concrete problem, resulting behavior and
observed validation. CI checks Linux and macOS, including coverage and source
installation. The stable required checks are `ci-gate` and `Lint PR title`.
Resolve review threads and squash-merge accepted changes; remove merged branches.
Default-branch safeguards should reject deletion, force-pushes and unsigned commits.
GitHub's squash merge produces the signed merge commit.

## Releases and eventual public availability

Creating or updating the private repository does not authorize release tags or a
visibility change. Release publication is deliberate and uses the existing tagged
workflow; see [installation and releases](docs/install.md). Before making the repo
public, enable private vulnerability reporting and review the available GitHub
code-scanning features. No paid security product is assumed by local development.
