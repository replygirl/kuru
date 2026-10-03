# Tasks

## 1. Bounded release download clients

- [x] 1.1 Move `CONNECT_TIMEOUT` and `READ_IDLE_TIMEOUT` to `archive.rs` as `pub(crate)` and import them in `bundle.rs`, and verify the delivery package builds with and without `tooling`
- [x] 1.2 Factor `archive::bounded_builder(connect, idle)` (the shared timeout shape), `archive::release_client` and `archive::download(client, url, limit)` with connect and read-idle bounds and phase contexts, and verify `read_asset` keeps its signature
- [x] 1.3 Factor `published::published_builder(connect, idle)` on `archive::bounded_builder`, `published::published_client`, `PublicGitHub::with_client` and `published::send`, keeping `no_proxy`, the redirect policy and user agent, with a read context in `bounded_body`, and verify `PublicGitHub::new` keeps its signature

## 2. Regression tests

- [x] 2.1 Add the test-only paced local HTTP fixture (tokio `TcpListener`, no new dependency) and verify it is bounded
- [x] 2.2 Add archive and published tests: a trickle past the old total succeeds under the idle bound; the old flat-total client fails the same trickle; a stalled body names the read phase; an unanswered and a refused connection name the send phase; build every test client from the production builders, and verify the phase assertions fail with the contexts removed and the idle-bound tests fail with the production timeout shape reverted

## 3. Checks

- [x] 3.1 Run the delivery package tests, `format:check`, `lint`, root `lint:windows`, `typecheck`, `lint:tooling` and `cospec -- validate --all --strict`, and verify each exits 0
