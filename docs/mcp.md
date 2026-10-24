# The Model Context Protocol, both ways

devd speaks MCP in two directions, and they are deliberately different code paths:

* **devd as a server** - `devd mcp` publishes devd's capability registry to an outside agent. That is
  the rest of this page up to "devd as a client".
* **devd as a client** - the `mcp.*` capabilities connect the built-in assistant to *other programs'*
  MCP servers, the way Claude Code uses the servers configured for it. Their tools join the assistant's
  tool list; they do not become devd capabilities, and they are not trusted like them.

## The MCP server

`devd mcp` serves the Model Context Protocol on stdin/stdout. It is the third
projection of the same registry, after the CLI and the application window, and it
makes the product's premise checkable: an agent driving `devd` reaches exactly
the same functions as a person, with exactly the same permission checks.

```bash
devd mcp                  # serve on stdio
devd mcp --client claude-code   # name the caller in the audit trail
devd mcp --list           # print the tool list and exit
```

A client configuration looks like this:

```json
{
  "mcpServers": {
    "devd": {
      "command": "devd",
      "args": ["mcp", "--client", "claude-code"]
    }
  }
}
```

## Five tools, and they do not grow

The obvious design is one MCP tool per capability. That is the design that gets
worse every time the product gets better: nearly 400 capabilities today, across
Git, runtimes, services, sites, TLS, the database, agents, operations and remote
servers, would be a tool list hundreds of entries long, loaded into every
conversation whether or not it is relevant, and regenerated whenever the registry
changed.

`devd` publishes **discovery** tools that read the registry instead:

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `devd_groups` | none | The domains, and how many capabilities each contains |
| `devd_capabilities` | `query`, `group`, `limit` | Search within a domain or across all of them |
| `devd_describe` | `id` | One capability: tier, risk flags, and the JSON Schema of its arguments |
| `devd_invoke` | `id`, `args`, `dry_run` | Run one by id |
| `devd_doctor` | none | The environment check |

A capability registered tomorrow is discoverable and invocable immediately,
with no change to this crate. A test asserts that two servers over registries of
different sizes publish an identical tool list — the surface is a constant, not
a function of the catalogue.

The intended call sequence is `devd_groups` → `devd_capabilities` → `devd_describe`
→ `devd_invoke`, and the `next_step` field of each result says so. That is what
the requirement meant by *discoverable domains rather than a huge always-loaded
tool list*.

## The agent actor is not negotiable

`McpServer::new` **discards** the actor in the context it is handed and installs
an agent actor. A caller cannot present a context claiming to be a human,
because a claim is not evidence, and the entire tier model exists so that the
decision does not rest on a claim. A test asserts that a server constructed with
a human context reports an agent actor.

The consequence is the product's central promise, now enforced on the agent
surface:

* An operation in a tier that requires approval is **refused**, and the refusal
  is written to the audit trail.
* An operation marked `human_only` is refused for an agent *even when an
  approval is presented*, because `Approval::Granted` is only ever produced by a
  human decision and the agent surface has no way to produce one.
* There is no general-purpose "run this command" capability, so a refused
  operation cannot be routed around by asking for something lower-level.

## What the agent gets instead of a success it did not earn

When an approval-gated operation is refused, the server does not simply fail. It
asks the registry for the **plan** — which changes nothing and is itself recorded
as a plan, not as an application — and returns it with the refusal:

```json
{
  "ok": false,
  "error": { "code": "approval_required", "message": "…" },
  "plan": {
    "summary": "reset Hard to HEAD",
    "steps": [
      "HEAD, the index AND the working tree are reset",
      "uncommitted changes to tracked files are DISCARDED"
    ],
    "destructive": true
  },
  "next_step": "A human must approve `git.reset` before it can run. Ask them to approve it in the devd application or with the devd CLI. An agent cannot approve its own operation, and you must not report this operation as having happened."
}
```

That field exists because the failure mode being defended against is an agent
that is *confident and wrong*: it will describe an operation it was refused as
though it had happened, unless it is told, in the result it is reading, not to.
An operation that cannot describe itself in advance gets no invented plan; the
`next_step` says so plainly.

`dry_run: true` on `devd_invoke` produces a plan and changes nothing, which is
the correct way for an agent to answer "what would this do?".

## Output is data, never an instruction

MCP results are inserted into a model's context, and a repository contains text
an attacker may control: commit messages, file names, branch names, diff
content. Nothing in this crate turns that text into a capability call. The tools
take a capability id and arguments; the only text that reaches the model is a
description the registry itself wrote, a value the registry returned, or an
error message. A `next_step` is generated by this crate, never echoed from a
repository.

Arguments are redacted before they are recorded, as on every other surface, so a
value the model supplies that looks like a credential does not reach the audit
file.

## Observed

Run against the release binary with a real repository, over a real pipe:

```
initialize        → devd 0.2.0, protocol 2025-06-18, 5 tools
tools/list        → devd_groups, devd_capabilities, devd_describe, devd_invoke, devd_doctor
devd_groups       → 64 capabilities in 12 domains
devd_describe     → git.reset: tier destructive, requires_approval true, arguments mode/repo/rev
devd_invoke       → git.status: ok, branch main, clean true
devd_invoke       → git.reset mode=hard: isError true, approval_required, plan handed over
devd_doctor       → ok, 8 checks including git 2.46.0
```

The audit trail after that session:

```
agent:claude-code  git.status   read         ok
agent:claude-code  git.reset    destructive  FAILED     <- the refusal
agent:claude-code  git.reset    destructive  ok         <- the plan, dry run
```

and `HEAD` was unchanged by the attempt.

## devd as a client: the `mcp.*` capabilities

The assistant can use tools from servers another program provides. The server configuration is
devd's; the tools are not. That is the whole design:

| Capability | Tier | Who | What it does |
| --- | --- | --- | --- |
| `mcp.list` | read | anyone | The configured servers: transport, enabled, trust, tool count and last error. Environment **names** only, never values. |
| `mcp.add` | write | a person | Validate and store a server: a `command` (stdio) or a `url` (http), arguments, environment, working directory, timeout. |
| `mcp.update` | write | a person | Change that definition; omitted fields keep their stored value (an empty environment clears it), and the next call reconnects. |
| `mcp.remove` | write | a person | Forget the server and delete its owner-only environment file. The program itself is not uninstalled. |
| `mcp.enable` | write | a person | Turn it on or off without losing the definition. |
| `mcp.trust` | write | a person | `ask`, `allow` or `deny` for agents. This is the security decision. |
| `mcp.test` | exec | anyone | Start or reach the server, perform the initialize handshake, ask for its tools, record the catalogue or the error. Calls no tool. |
| `mcp.call` | exec | a person directly; an agent after approval | Call one tool and return its result as data. |
| `mcp.call_trusted` | exec | the assistant | The path used for a server marked `allow`; its handler refuses an agent unless the stored trust is exactly `allow`. |
| `mcp.import` | write | a person | Read `~/.claude.json`, Claude Desktop's `claude_desktop_config.json`, `~/.cursor/mcp.json` or a project `.mcp.json` and add the servers it finds. |

The assistant sees each tool as `mcp__<server>__<tool>`, with the server's own description and input
schema. It calls it through `devd_invoke` like every other operation, so the call is validated by the
registry, recorded in the audit trail with its arguments redacted, and appears in the approval queue
when it needs a person.

### Trust is enforced where the call runs

| Trust | An agent's call |
| --- | --- |
| `ask` (default) | `mcp.call` is refused by the registry with `approval_required`; the plan names the server, the tool and the redacted arguments, and a person approves or declines it. |
| `allow` | The assistant uses `mcp.call_trusted`, whose handler re-reads the stored trust and refuses anything that is not exactly `allow`. A prompt cannot grant this; only `mcp.trust` can. |
| `deny` | The tools are not offered, and both call paths refuse an agent. |

A person calling `mcp.call` directly (CLI, window) is not asked: they chose the server and they are the
one acting. Only the configuration capabilities and `mcp.trust` are human-only, so an agent can never
add a server for devd to run or widen its own access.

### Transports

* **stdio** - devd starts `command` with `args`, speaks newline-delimited JSON-RPC over its pipes, and
  stops the whole process tree when the connection is dropped or a request times out. `cwd` is optional.
* **http** - the Streamable HTTP transport: one POST per message, a JSON or `text/event-stream`
  response, and the `mcp-session-id` the server returns is carried on later requests. Redirects are
  refused so a credential cannot be forwarded to a third host.
* The older two-endpoint HTTP+SSE transport is **not** implemented. A server that needs it is not
  half-supported; it is recorded in `limitations.md`.

### Secrets and the environment

A stdio server is started with a small, named environment (`PATH`, `SystemRoot`, `APPDATA`,
`USERPROFILE`, `TEMP`, the proxy variables and the platform equivalents) and **not** devd's environment,
so `.env` values, provider keys and session tokens are not inherited. Anything the server needs is given
with `env` on `mcp.add`; the values are written to `<devd home>/mcp/<id>.env.json`, readable only by the
current user, and never stored in the database, the audit trail, logs or API responses. `mcp.list`
returns their names only.

Arguments are passed as an argument vector, never through a shell, so a value containing `&`, `;` or a
quote is not interpreted. Tool output is data: the assistant's prompt says so, and a result the server
marks `isError` is recorded as a failed tool result, not a successful one.

### Bounds

* A request's timeout is 5-600 seconds (default 120). On timeout the process tree is killed and the
  error names the method and the server's last standard-error lines.
* `tools/list` follows pagination cursors, capped at 50 pages.
* A result is capped at 200,000 characters before the conversation's own, smaller, cap applies.
* Connections are pooled per server, keyed on the configuration's `updated_at`; a call is **never**
  retried after a failure, because a tool may already have had its side effect.
* Tools are discovered at the start of an assistant run. A server that cannot be reached is reported
  as a notice in the conversation and recorded on its row (`last_error`); it does not stop the run.

### Observed

`devd-mcp/tests/client.rs` starts a real child process (the crate's `fake-mcp` bin), performs the real
handshake, discovers `echo` and `fail`, calls them, and asserts that the server's `isError` is
preserved. A second test drives the actual registry: an agent's `mcp.call` on an `ask` server returns
`ApprovalRequired` without reaching the handler, a person's call succeeds, `mcp.call_trusted` refuses an
agent for a non-`allow` server and succeeds for an `allow` one, and `deny` refuses both. A third checks
that external tools are prefixed, and that a denied server is not offered.

## Testing

24 unit tests and 3 end-to-end tests in `devd-mcp` (26 unit tests including the argument-redaction and address cases):

* the protocol: `initialize` (including echoing an unknown version), notifications
  getting no reply, an unknown method, a missing required argument;
* the surface: five tools, each with a schema and a real description; and the
  assertion that the list does not grow with the registry;
* the actor: a human context is discarded;
* discovery: group counts, search, domain filtering, describe;
* refusal: a destructive operation does not run and its plan is returned; an
  operation that cannot plan says so; a `human_only` operation is refused; a dry
  run changes nothing;
* `devd_doctor`, including an appended check and the `ok` computation;
* the client, against a real child process: discovery, calls, the `isError`
  result, the trust/approval boundary, and the tool naming.

The refusal tests assert against a counter incremented by the handler, so
"nothing ran" is a measurement rather than an inference.
