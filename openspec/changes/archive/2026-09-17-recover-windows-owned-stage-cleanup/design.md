## Context

A successful activation now reports a failed private-stage close. Windows can transiently deny the exact private stage or an isolated fixture's cache-binary deletion with OS32, while a changed path must never be treated as the original owned object.

## Goals / Non-Goals

**Goals:** Preserve exact outer and child identities, retry only the native sharing violation within two seconds, retain the cache lease throughout cleanup, and make fixture cache invalidation equally identity-checked.

**Non-Goals:** No Win32 API, generic deletion framework, public timeout knob, publication retry, process lifecycle, or production cache-corruption behavior change.

## Decisions

`PrivateTemp::close` disarms `TempDir` before validating the retained outer and private-child paths. The existing checked `Directory::remove_tree` removes the child; after an uncertain OS32 result, reopening must prove the same child or confirmed absence under the unchanged outer identity. Only then may it retry. The outer container is removed only with raw `remove_dir` after the child is absent and its identity is revalidated. Non-OS32 and identity changes fail immediately with preserved native causes on bounded expiry.

Fixture-only binary invalidation reopens and proves the recorded binary identity before each deletion attempt, retrying only OS32. A private test observer runs only after an actual child-removal OS32 so it can prove release recovery and replacement refusal without timing sleeps.

## Risks / Trade-offs

A published engine can still yield an error when a persistent handle prevents cleanup. That error retains the successful-publication context and native cause, preserving the exact stage for inspection rather than treating a replacement as disposable.

## Operational surface

The correction affects Windows native cache-stage cleanup and Windows-only fixture recovery. It changes no binary payload, cache format, bind address, credentials, or CI topology.
