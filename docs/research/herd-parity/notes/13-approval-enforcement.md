# 13 — Approval enforcement (X01–X03)

**Claim under test.** README: "Every interface calls the same dispatcher" (§How it works).

**Evidence (read, not executed).**
- Central path: `Registry::dispatch` checks `Tier::Destructive|Remote|Admin` and `human_only` (`crates/devd-core/src/registry.rs:156-263`). `/api/invoke`, `approve` and `deny` use it (`crates/devd-server/src/api.rs:176-340`). Project routes use `dispatch_project` (`api.rs:855-920`).
- Bypass: `session_input` (`api.rs:601`), `terminal_input` (`:619`), `session_resize` (`:636`), `session_rename` (`:675`), `session_stop` (`:687`), `terminal_stop` (`:700`), `session_dismiss` (`:711`) and `task_cancel` (`:782`) call `state.workspace.sessions()` or `orchestrator()` directly.
- The matching capability for the dismiss route is `session.dismiss`, tier `destructive` (`devd capabilities`).

**Documented exceptions.** The comment above `terminal_harnesses` in `api.rs` records that polling output, keystrokes and resizing are deliberately not capability calls (an audit row per frame is noise). `session_input`, `terminal_input`, `session_resize`, `terminal_resize` stay direct. The defect is that the same comment promised *every other mutating operation* goes through the registry, which was untrue for start, stop, rename, dismiss and cancel.

**Status: fixed in the working tree (M0).** `session.start|stop|rename|dismiss` and `agent.cancel` are now dispatched (`api.rs` `dispatch_route`, `start_session`); the window dismisses through `run("session.dismiss")`, which shows the plan and asks. Tests added in `crates/devd-server/tests/http.rs`: a refused `DELETE /api/sessions/:id` leaves the session in place, audit rows exist for start/stop/dismiss, approval completes the dismissal, and a source guard fails if `api.rs` calls those subsystem methods directly. See the test result in the final report; not committed.

**Impact.** The HTTP API requires the per-process bearer token on loopback, so this is not a remote hole. It is an inconsistency: the window can dismiss a session without the approval dialog and without an audit row, contradicting the documented guarantee. Agents reach these operations through MCP's registry path, so the "an agent cannot approve its own action" invariant is not shown to be affected; the audit-completeness and single-implementation claims do not hold for these routes.

**Herd comparison (X03).** REA fuse read of `Herd.exe`: RunAsNode enabled, ASAR integrity validation not enforced, OnlyLoadAppFromAsar disabled. Informational only.

**Uncertain.** Other direct paths outside lines 601–727 (U-12). Whether `session.input` typing is intentionally exempt from audit; if so, document it.

**Regression tests.** A route-vs-registry enumerator (M0 acceptance #3); `session.dismiss` over HTTP without a token returns `approval_required`; audit rows exist for stop, rename and cancel.
