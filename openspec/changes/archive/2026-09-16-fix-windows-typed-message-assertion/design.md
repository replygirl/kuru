## Context

The delivery acceptance fixture opens the installed memory read-only and finds
the original user transcript row. It retained a direct `Message.content` field
read after typed messages moved textual content into ordered blocks.

## Goals / Non-Goals

**Goals:**

- Compile the Windows delivery fixture against the typed message contract.
- Preserve an exact assertion that the original input is one plain-text message.

**Non-Goals:**

- Change transcript storage, migration, production delivery behavior, or test
  topology.

## Decisions

- Use `message.plain_text() == Some(marker.as_str())` alongside the existing
  `user` role assertion. It accepts only the intended one-text projection and
  does not flatten arbitrary typed blocks.

## Operational surface

The affected executable is the existing Windows native delivery test. It uses
the package-owned embedded Dolt fixture; no runner settings, credentials,
network endpoint, or binary selection changes.

## Risks / Trade-offs

- A generic projection could hide a changed message shape. Restricting the
  assertion to `plain_text` keeps the one-text contract explicit.
