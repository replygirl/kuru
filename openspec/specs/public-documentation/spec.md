# public-documentation Specification

## Purpose
Provide accessible, curated Kuru documentation with accurate setup and reference
guides, searchable static output, and interactive previews of the framework
models without publishing repository-only records or private state.

## Requirements

### Requirement: Curated public content

The documentation site MUST publish only deliberately authored product documentation from `apps/kuru-docs`. It MUST NOT automatically publish repository engineering notes, local paths, credentials, or account-specific verification records.

#### Scenario: Production build
- **WHEN** the documentation production build runs
- **THEN** HTML, search data, and machine-readable documentation contain the curated user pages and no repository-only records

### Requirement: Usable product reference

The site SHALL document working installation, authentication, terminal controls, framework concepts, memory, dreaming, configuration precedence, tools, MCP, and A2A with examples matching the current implementation.

#### Scenario: First use
- **WHEN** a visitor follows the first conversation guide
- **THEN** they can run the offline demo without credentials and find model discovery and authenticated setup instructions

### Requirement: Accessible navigation and product identity

The site SHALL provide dark and light themes, responsive navigation, local search, visible keyboard focus, readable contrast, and framework graphics with equivalent text. Decorative motion MUST respect reduced-motion preferences and MUST NOT move article content.

#### Scenario: Narrow viewport and keyboard input
- **WHEN** a visitor uses a narrow viewport or navigates with a keyboard
- **THEN** navigation, framework selection, content, and search remain available without horizontal page overflow

### Requirement: Project Pages compatibility

The application SHALL build a static site for the `/kuru/` project path with correct internal links, search assets, and direct page navigation.

#### Scenario: Production preview
- **WHEN** the built site is served at `/kuru/`
- **THEN** landing pages, nested reference pages, and local search load successfully
