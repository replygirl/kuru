# Spec Delta

## MODIFIED Requirements

### Requirement: Safe local extraction

Runtime provisioning MUST retain bounded archive and payload validation, private staging, cancellation cleanup, an exact-version check of every freshly extracted engine before its activation, and atomic activation. Existing cache verification MUST retain checked directory and payload handles while reading every byte for the pinned digests, and MUST revalidate their names and identities before returning the verified pathname; it launches no process. This is a deliberate trade: an existing cached executable whose bytes match the pinned digest yet cannot run, or reports another version, is no longer detected by that verification and surfaces instead when the database server fails to start; the install-time probe and the Dolt-version-keyed cache directory are the mitigations. An explicitly configured `dolt_binary` is not a managed cache and is outside this extraction requirement; it remains version-probed on every open. Independent warm-cache verification SHALL proceed without the exclusive installation lock. An absent cache MUST be rechecked after acquiring that lock, and extraction and publication MUST retain it through their existing checked completion or recovery boundary, including the exact-version probe of the freshly extracted engine before it is activated. Runtime provisioning SHALL have no runtime HTTP engine-download path.

#### Scenario: Corrupt cache or archive
- **WHEN** an embedded archive or existing cache fails integrity validation
- **THEN** Kuru executes no unverified payload and preserves existing installed data

#### Scenario: Concurrent warm cache verification
- **WHEN** independent callers open the same existing valid cache while the installation lock is held or another warm verification is running
- **THEN** each caller verifies the complete pinned payload without waiting for the installation lock, and none re-probes its already-installed version

#### Scenario: Concurrent or cancelled first use
- **WHEN** extraction callers race or a caller is cancelled
- **THEN** staging remains owned until work stops and another caller observes only a fully verified, version-probed, activated runtime
