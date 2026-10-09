# Proposal

## Why

Native CI found new acceptance fixtures failing before proving their intended
coverage: the HTTPS peer has ambiguous certificate names, the permission test
mistakes the permanent footer for an open inspector, and ConPTY projects a
combining accent differently from the stored prompt.

## What Changes

- Align the shared notice HTTPS peer with the existing synthetic MCP server
  certificate profile and retain bounded fixture transport failure diagnostics.
- Match the permission inspector's actual title when waiting for its dismissal.
- Correct only native recall frame synchronization while preserving the exact
  Unicode input, submitted provider requests, and durable transcript assertions.
- Verify the affected fixture contracts through focused checks and native CI.

## Impact

Only existing TUI acceptance tests and their shared support files change.
Production behavior, trust and TLS checks, fixture deadlines, source coverage
inventory, tool pins, lockfiles, and the 95% minimum remain unchanged.
