# Authentication & models

Kuru has three inference providers. The peer pool, memories, and tool permissions belong to Kuru regardless of which provider you choose.

| Provider    | Authentication                     | Use                                                        |
| ----------- | ---------------------------------- | ---------------------------------------------------------- |
| `codex`     | ChatGPT browser or device sign-in  | Direct ChatGPT subscription requests; the default provider |
| `responses` | API key in an environment variable | Direct OpenAI Responses API access                         |
| `demo`      | None                               | Deterministic offline exploration                          |

## Sign in with ChatGPT

Kuru handles sign-in and sends model requests directly to OpenAI. Install Kuru,
then sign in with your ChatGPT account; no Codex CLI, Node or npm installation
is required.

```sh
kuru login
kuru auth
kuru models
kuru
```

`kuru login` opens your browser and waits for the local authorization callback.
To open the printed URL yourself, use:

```sh
kuru login --no-browser
```

Browser login uses the local callback port `1455`. If that port is occupied,
Kuru reports the conflict; use device authorization instead. If launching the
browser fails, the printed URL remains available for manual sign-in.

For device authorization:

```sh
kuru login --device
```

Kuru stores its session in the private `auth/openai` directory beneath its data
directory. If you choose `--data-dir` or `KURU_DATA_DIR`, use that same location
for login and chat. Keep it outside the project tool root. Kuru does not read
or copy credentials from another application's store.

`kuru auth` prints redacted local status as JSON, without creating credentials
or opening project memory. It is not a live access check. `kuru logout` clears
Kuru's ChatGPT credentials; an API key supplied through the environment remains
unchanged. Sessions refresh when needed. If login or refresh fails, follow the
reported sign-in guidance; Kuru does not switch providers automatically.

## Subscription compatibility

`codex` is Kuru's native compatibility route for a ChatGPT subscription. It
uses client `app_EMoamEEZ73f0CkXaXp7hrann`, issuer `https://auth.openai.com`,
the fixed subscription base `https://chatgpt.com/backend-api/codex`, and catalog
compatibility version `0.154.0`. Browser OAuth uses `/oauth/authorize` and
`/oauth/token`; device authorization uses the issuer's `/api/accounts/deviceauth`
paths. These are fixed connector values, not configuration options or a public
OpenAI contract for Kuru, so availability can change or stop working.

`responses` remains the separate, explicit API-key route. It alone honors
`api_base` and `api_key_env`; Kuru never sends a ChatGPT session to that
endpoint. `kuru logout` removes Kuru's local ChatGPT credentials only. No
remote OAuth-revocation endpoint is established for this route, so logout does
not claim to revoke a remote grant.

When this compatibility contract changes, update the connector literals and
their independent loopback assertions together: `auth/tests.rs` covers browser
and device OAuth paths, and `providers/subscription_tests.rs` covers the fixed
subscription base and catalog version. Run `kuru auth` and `kuru models` to
inspect local status and the current catalog; a user-participating live check is
separate from these deterministic tests.

For existing configurations, remove `codex_command`; it no longer selects an
external executable. Keep `provider = "codex"` and run `kuru login` to establish
Kuru's own session.

## Discover models and effort

```sh
kuru --provider codex models
```

Kuru reads the current provider catalog and preserves its advertised model IDs and reasoning effort values. Availability depends on the provider and your account. Use the catalog rather than a fixed list in documentation.

In the terminal, <kbd>F2</kbd> selects a model and <kbd>F3</kbd> selects effort. Both pickers accept a typed filter. Selecting `default` clears the explicit effort setting. To choose values for one invocation:

```sh
kuru --provider codex --model MODEL_ID --effort EFFORT
```

Replace `MODEL_ID` and `EFFORT` with advertised values. Terminal choices are remembered for the project; command-line overrides are temporary. See [configuration precedence](/reference/configuration#precedence).

## Use the Responses API

Make an API key available through `OPENAI_API_KEY` using your environment or secret manager, then discover model IDs:

```sh
kuru --provider responses models
kuru --provider responses --model MODEL_ID
```

This provider calls the [OpenAI Responses API](https://developers.openai.com/api/reference/resources/responses/). Its model catalog does not advertise a default chat model or reasoning effort capabilities, so select a suitable model explicitly. An empty effort list does not mean every effort is supported; provider validation errors remain authoritative.

For this provider, `api_key_env` changes the environment variable name, and
`api_base` changes the API base URL. Put the variable name in configuration,
never the key itself. ChatGPT credentials are not sent to this configurable
endpoint, and setting an API key does not change the selected provider.

## Try the pool offline

```sh
kuru --provider demo --mode jungian
```

The demo provider requires neither network access nor credentials. Kuru's bundled Dolt engine supports a first offline conversation with an empty cache. The demo lets you explore the interface, framework choices, memory, and session controls. Its predictable responses do not test live model access.
