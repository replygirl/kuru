# Frequently asked questions

## Does `kuru doctor` test my account or API key?

No. It reads Kuru's own local ChatGPT sign-in status without showing account details. For the explicit Responses route, it may report whether the selected environment variable is non-empty, but it does not validate the key or make a provider request. An unapproved repository-selected variable remains unverified until its applicable workspace authority is reviewed. The two routes do not fall back to each other.

## Can the doctor start or repair memory?

No. It checks the current project's bounded activation metadata and can query an already-published compatible memory owner through a read-only local connection. It does not create a store, start or elect a service, provision Dolt, migrate, or repair files. A valid cold project therefore has unverified deeper SQL health.

## Does Kuru delete old conversations automatically?

No. Project histories and notes do not expire automatically. Selected-note forgetting changes active notes while retaining prior Dolt revisions. Project purge removes the selected project's managed current store and revisions but does not promise secure physical erasure and does not remove unrelated projects, exports, backups, original shared migration inputs, or the engine cache. See [Parts, relationships & memory](../concepts/memory).

## How large is the download?

It varies by platform and release archive. As a dated example, the v0.9.0 release assets published on 2026-09-25 were about 45–49 MiB compressed across supported targets. That is the complete release archive size for those specific assets, not the installed executable size or the expanded engine-cache size. See the [v0.9.0 release assets](https://github.com/replygirl/kuru/releases/tag/v0.9.0) and [Installation & updates](./installation) for current target details.

## Does shell execution run in a sandbox?

No. An approved shell command has the user's process authority. Workspace trust reviews which configured authority is allowed; it does not confine the operating system process. See the [threat model](../concepts/threat-model).
