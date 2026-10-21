# 11 — Agents, MCP, IDE (I01–I06, D08, P07)

**Evidence.** Herd's `mcp:install` registers its server with eight agents; Studio uses a helper on `ws://127.0.0.1:4421` (prior doc). DEVD: `devd mcp` (5 discovery tools, `crates/devd-mcp/src/lib.rs`), real PTY sessions, delegation and the team. `docs/mcp.md` shows a manual `mcpServers` JSON snippet (line 18). No registration command was found by grep, and there is no default-IDE concept.

**Classification.** MCP server: implemented. Client registration: not implemented. Studio: intentional divergence. Tunnel: not implemented.

**Tests to add.** `mcp install` writes only the devd entry, preserves other config and is idempotent; uninstall removes only that entry; each agent's config path is resolved per OS.
