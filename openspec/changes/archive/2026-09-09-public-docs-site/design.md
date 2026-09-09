## Context

Cospec uses VitePress with a default-theme extension, local search, and llms output. Kuru has an established ink, mint, lilac, blue, and amber terminal palette, four distinct framework portraits, and user documentation mixed with engineering records.

## Goals / Non-Goals

**Goals:** Follow the requirements in `specs/public-documentation/spec.md` with a small maintainable site. Retain ordinary VitePress navigation and search, author concise task-oriented pages, and make the product recognizable through restrained typography and code-native framework graphics.

**Non-Goals:** A new frontend framework, commercial fonts, account/model inventories, generated screenshots, runtime changes, deployment ownership, or publishing repository implementation records.

## Decisions

1. Extend VitePress instead of replacing its layout. Default accessible navigation, search, and document structure reduce custom interaction code; a small framework component and CSS provide identity.
2. Use an explicit `apps/kuru-docs` content boundary rather than the repository root. Generated HTML, search data, and llms files then share the same curated source set.
3. Use exact versions requested for parity with the current Cospec architecture: VitePress 2.0.0-alpha.20 and vitepress-plugin-llms 1.13.5. Root owns installation and the workspace lockfile.
4. Use fixed ASCII portraits and a very slow optional color cycle rather than moving particles or typing effects. Text explains each framework; the graphic is decorative.
5. Keep installation examples honest about private repository access and release availability. Do not imply a release exists until the coordinated release work establishes it.
6. Keep source-engineering docs in place and write curated user pages rather than blindly cloning the existing docs tree. Public pages own their user-facing instructions.

## Risks / Trade-offs

- [Prerelease VitePress changes] → Exact pins, production build verification, and browser interaction checks.
- [Private information in generated output] → Explicit content root plus output inspection, including llms artifacts.
- [Subpath asset failures] → `/kuru/` base and production-preview navigation/search checks.
- [Theme contrast or animation discomfort] → Test both themes and reduced motion; use static geometry and system fonts.

## Operational surface

The output is a static browser site under `/kuru/`; no application server, credentials, or database is shipped. Local development/preview binds to loopback by default. App-owned Node 26.8.2/npm dependencies run the pinned VitePress build through mise on the host or CI runner, without a container requirement or architecture-specific site output. GitHub Pages upload/deployment permissions and hosted verification are owned by the sibling delivery change. Local search reads built public indexes and has no external search service or connection pool.
