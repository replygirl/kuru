# Proposal

## Why

The M1 protocol 1.13 hello explicitly serializes restored history provenance. The
unchanged starter-token fixture omitted `history_scope:null` from its exact JSON
expectation and failed Ubuntu coverage partition 4 on the published M1 head.

## What Changes

- Update only the expected no-starter-token JSON in the existing memory service
  test to include the declared null history field in struct order.
- Retain every parsing, exact-wire, token privacy and authority assertion.

## Impact

Only `packages/kuru-memory/src/service.rs` test expectations change. Production
serialization, protocol version, schema, budgets and CI coverage remain unchanged.
Verify the existing starter-token and pinned protocol-surface tests, affected
static checks, then fresh full PR and exact-main CI.
