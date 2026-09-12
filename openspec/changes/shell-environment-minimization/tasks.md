## 1. Finite environment projection

- [x] 1.1 Add private platform-specific shell environment projections in
  `packages/kuru-connectors/src/tools.rs` using the exact documented names,
  Unix case-sensitive and Windows ordinal case-insensitive matching, canonical
  output keys, deterministic rejection of duplicate inherited compatibility
  names, ignored excluded/native-derived input duplicates and no public/config API.
- [ ] 1.2 Add pure fake-map regressions for exact allowed/excluded values,
  non-Unicode values, Windows case ambiguity, native-derived system-shell paths,
  inherited `PATHEXT` and its absent-value fallback; verify the tests fail while
  the shell still copies the complete parent environment and pass after the fix.

## 2. Native built-in shell launch

- [x] 2.1 Apply `env_clear` plus the finite projection to only the Unix built-in
  `sh -c` launch, preserving retained-root revalidation, cwd, output/deadline
  bounds and process-group cleanup.
- [x] 2.2 Replace the Windows built-in shell's complete parent copy with the
  finite projection, retaining absolute stock PowerShell 5.1,
  `-NoProfile -NonInteractive`, native-derived `SystemRoot`/`WINDIR`/`ComSpec`,
  inherited-or-fallback `PATHEXT`, and absent `PSModulePath`.
- [ ] 2.3 Spawn isolated child fixtures with fake parent values and exercise the
  real `ToolHost` shell on Unix and native Windows; verify allowed values and
  controlled bare commands work, excluded sentinels are absent, result values
  are not dumped, and the existing timeout, root-replacement and platform cleanup
  assertions remain satisfied without adding a Unix lifecycle guarantee.

## 3. Separate MCP contract and documentation

- [x] 3.1 Extend a real local stdio MCP fixture to observe one fake inherited
  non-allowlisted arbitrary sentinel and one explicit `McpConfig.env` override;
  verify no shell projection is introduced into shared RPC or native process
  code.
- [x] 3.2 Update the shell tool description, `docs/protocols.md` and
  `apps/kuru-docs/reference/tools.md` to name the finite inherited-variable
  reduction while retaining the explicit process-authority and
  no-filesystem-sandbox language.

## 4. Verification

- [ ] 4.1 Run focused shell/projection/MCP tests through the owning connector
  mise task during implementation and run the applicable cases on native
  Windows; record exact commands, exits and fixture counts without substituting
  cross-compilation for runtime evidence.
- [x] 4.2 Run connector lint/typecheck, documentation checks and strict cospec
  validation, then the single coordinated workspace coverage task; verify every
  gate exits zero and coverage remains at least 90 percent without exclusions.
