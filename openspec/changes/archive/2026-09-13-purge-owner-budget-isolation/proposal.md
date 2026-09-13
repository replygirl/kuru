## Why

The live-owner purge refusal test must exercise the refusal path even when a Windows host takes longer than one second to initialize its isolated memory store.

## What Changes

- Move the test-only one-second timeout assignment to immediately before `MemoryStore::purge`, while keeping every existing refusal and preservation assertion.

## Impact

Touches one real-memory fixture and its verification record; no production behavior or CI policy changes.
