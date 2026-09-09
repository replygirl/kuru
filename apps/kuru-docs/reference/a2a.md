# A2A peers

Kuru supports a bounded, nonstreaming A2A 1.0 JSON-RPC `SendMessage` flow at its external boundary. Internal parts communicate through typed envelopes and mailboxes; they do not make loopback HTTP calls to each other.

## Configure an external peer

```toml
[external_agents]
research_peer = "https://example.com/a2a"
```

Replace the example with an explicit trusted endpoint. Up to 64 endpoints are accepted; embedded URL credentials and fragments are rejected. The initial implementation has no per-agent outbound credential configuration. The endpoint must accept the supported transport or use an appropriate local gateway.

## Start local ingress

Supply a bearer token of at least 16 characters through the `KURU_A2A_TOKEN` environment variable, then start the service:

```sh
kuru --provider demo serve
```

The default listener is `127.0.0.1:7437`. `--bind` selects another loopback address and `--token-env` selects another environment variable:

```sh
kuru --provider demo serve --bind 127.0.0.1:7438 --token-env MY_A2A_TOKEN
```

The service requires a loopback bind and authenticated requests. Use an authenticated gateway if you need remote exposure.

## Message behavior

Requests use `messageId`, `contextId`, `ROLE_USER`, and text parts, with the `A2A-Version: 1.0` header. Responses may contain a message or a task. Terminal task text is extracted from status messages and artifacts.

Protocol errors, malformed responses, excessive payloads, and timeouts become visible call errors.

## Current scope

Streaming, push notifications, remote task polling, and automatic agent discovery are outside this subset. A configured endpoint provides a deliberate connection to another agent; it does not create an implicit trust relationship or merge private part memories.

See [parts and memory](/concepts/memory) for the boundaries within the local pool.
