# 08 — Workspaces (P13)

**Evidence.** Herd: `workspace` creates a worktree on its own branch served at its own `.test` domain with its own certificate, with merge, delete, open and switch. DEVD: 4 `git.worktree.*` capabilities (add, list, prune, remove); the agent team runs implementers in isolated copies (`crates/devd-workspace/src/orchestrate.rs`, parallel-worktree test in `crates/devd-agent/tests/agent.rs`). No capability serves a worktree on a domain.

**Classification.** Backend-only / partial. Isolation exists for agents; the site-per-branch developer workflow does not.

**Dependency.** Needs M3 (upstream-aware sites) to serve arbitrary stacks.

**Tests to add.** Creating a workspace issues a distinct certificate and hostname; delete removes both; merge refuses on a dirty tree (reuse `git.switch.check`).
