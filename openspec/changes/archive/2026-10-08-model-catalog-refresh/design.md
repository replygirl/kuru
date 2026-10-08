# Design

## Context

Startup and `models` already fetch live account catalogs. Login currently only persists credentials. The embedded catalog enriches exact route-and-ID pairs rather than deciding availability.

## Goals / Non-Goals

Use the existing native transport and preserve the proposal's scope. Do not add a disk cache, alias resolver, inference calls, or authentication fallback.

## Decisions

- Keep fresh discovery on each open rather than adding a daily persistent cache: the current behavior already makes new models available immediately and avoids stale account or route data.
- Perform discovery after successful login through a connector-owned fixed subscription entrypoint rather than workspace configuration: account operations must remain independent of unrelated authority.
- Preserve login success if discovery fails, with a fixed safe notice rather than relaying provider error text.
- Add API limits only on the API route and label subscription API-equivalent prices. Leave unverified subscription limits and tokenizer mappings absent instead of copying facts across routes.

## Operational surface

The CLI prints discovered model IDs after login. It adds no listener, process, required secret, or connection topology; existing browser/device login and bounded catalog HTTP remain unchanged. Existing Kuru authentication is the only subscription authority. The optional native acceptance command only lists models and sends no inference.

## Integration contract

`kuru-connectors` owns the fixed ChatGPT `/models` request and its compatibility query, OAuth refresh and safe parsing. Subscription `models[].slug` and advertised effort strings are preserved, including unknown values. Public Responses `data[].id` remains a separate selected-provider contract. No provider prompt or tool suggestions become Kuru authority.

## Risks / Trade-offs

- Login catalog service outage → successful authentication remains committed and retry guidance names `kuru models`.
- Public metadata is not subscription availability or billing → exact route keys and API-equivalent labels preserve that distinction.
- Unknown new models lack verified prices/tokenizers → retain supported live discovery and labelled fallback estimates.
