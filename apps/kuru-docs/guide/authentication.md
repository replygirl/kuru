# Authentication & models

Kuru has three inference providers. The peer pool, memories, and tool permissions belong to Kuru regardless of which provider you choose.

| Provider    | Authentication                     | Use                                                             |
| ----------- | ---------------------------------- | --------------------------------------------------------------- |
| `codex`     | Supported Codex sign-in            | OpenAI inference through Codex app-server; the default provider |
| `responses` | API key in an environment variable | Direct OpenAI Responses API access                              |
| `demo`      | None                               | Deterministic offline exploration                               |

## Sign in through Codex

Have the `codex` executable available on your `PATH`. The repository's mise toolchain supplies its pinned supported version; `codex_command` can select another executable path.

```sh
kuru login
kuru auth
kuru models
kuru
```

Kuru forwards login, status, and logout to Codex. Codex manages the credentials; Kuru does not copy authentication tokens. Follow [OpenAI's authentication instructions](https://learn.chatgpt.com/docs/auth?surface=app) for supported account and sign-in options.

For device authorization:

```sh
kuru login --device
```

`kuru logout` signs out through Codex. If login fails, resolve the reported Codex error before retrying Kuru.

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

`api_key_env` changes the environment variable name, and `api_base` changes the API base URL. Put the variable name in configuration, never the key itself.

## Try the pool offline

```sh
kuru --provider demo --mode jungian
```

The demo provider requires neither network access nor credentials. Kuru's bundled Dolt engine supports a first offline conversation with an empty cache. The demo lets you explore the interface, framework choices, memory, and session controls. Its predictable responses do not test live model access.
