## 1. Expanded credential projection [critical]

- [x] 1.1 @regression (agent) run the connector redaction unit suite with synthetic Slack, GitLab, JWT, URL-userinfo and bare assignment inputs -> `cargo test -p kuru-connectors redaction:: --locked`: 16 passed; whole and every-byte projection replace each synthetic form with the exact marker
- [x] 1.2 @unit (agent) exercise text and structured JSON key/value projection plus Display/Debug failure projection -> `cargo test -p kuru-connectors redaction:: --locked`: 16 passed; structured JSON remains valid and marker/error-boundary coverage remains green
- [x] 1.3 @eval (agent) review expanded detector controls against the documented finite inventory -> implementation uses explicit prefixes, a JOSE-shaped JWT header and bounded RFC-style URL syntax; it adds no entropy heuristic or arbitrary-secret claim

## 2. Projection limits remain truthful

- [x] 2.1 @equivalence (agent) split every added form at each byte and as one-byte chunks -> `cargo test -p kuru-connectors redaction:: --locked`: 16 passed, including every-byte and long cross-chunk controls without a recognized fragment
- [x] 2.2 @unit (agent) run ordinary source, URL, assignment and malformed-JWT controls -> `cargo test -p kuru-connectors redaction:: --locked`: 16 passed; ordinary URL, assignment, short-token and malformed-JWT controls remain exact

## 3. Public contract and reference

- [x] 3.1 @unit (agent) compile connector callers against the public text/JSON projection seam -> focused connector test compilation passed; `project_text`, `project_json`, and `ProjectionError` are re-exported without changing `ToolHost::execute`
- [x] 3.2 @manual (agent) inspect the protocol reference detector inventory and limits -> `docs/protocols.md` names the finite detector forms, bounded URL/JWT rules, false-positive controls and existing projection limits
