## Context

The selected instrumented Cargo executable carries removable compiler metadata. The fixture applied a selected-input bound derived from the 128 MiB shipping cap before it could make and strip a private copy. The measured 171,024,280-byte instrumented input exceeds that old bound, while a distinct copy stripped with Apple `strip -u -r` is 118,213,992 bytes.

## Decisions

Use an independent 192 MiB test-only limit for reading the selected debug or instrumented Cargo input. Keep the held source's identity, length and digest checks, the independent bounded copy, and the final 128 MiB shipping limit. The runtime installation, update and profile assertions remain the proof that the resulting private copy is usable.

## Risks / Trade-offs

An instrumented executable larger than 192 MiB will still fail before acceptance. That is a bounded failure with a clear selected-input cause; any future adjustment needs measured evidence. The unchanged final 128 MiB limit still rejects a copy whose removable metadata does not strip far enough.

## Operational surface

Only the native CI runner's selected Cargo test executable is affected. The fixture uses its existing private copy and packaged offline runtime path; bind addresses, secrets, executable targets, and installed release bytes are unchanged.
