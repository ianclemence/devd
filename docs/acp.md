# ACP: devd as another agent's builder

The Agent Client Protocol (ACP) lets an editor or an engine drive an agent over
stdin/stdout. MCP is the opposite direction: an agent drives devd's capabilities.
`devd acp` exposes devd as the builder, and every prompt becomes an ordinary devd
conversation, so tool calls keep the same registry, approvals and audit chain as
the window and the command line.

```text
craft-anything / Zed  --session/new, session/prompt-->  devd acp
                      <--session/update, request_permission--
```

## Run it

```bash
devd acp           # serve JSON-RPC on stdin/stdout
devd acp --check   # print the advertised capabilities and exit
```

An ACP client starts it like any other agent:

```json
{
  "command": "devd",
  "args": ["acp"],
  "env": { "DEVD_HOME": "D:/devd" }
}
```

Configure a provider and model first (`devd provider`, `devd models`), as for
`devd chat`. ACP adds no second place for keys.

## Methods

| Method | What it does |
|---|---|
| `initialize` | Reports protocol version 1, text prompts only, no session load, no ACP-side MCP transports. |
| `authenticate` | Accepts; authentication is devd's provider configuration. |
| `session/new` | Takes an absolute `cwd` and returns an opaque `sessionId`. |
| `session/prompt` | Sends the text to the built-in assistant, streams updates, and ends with a `stopReason`. |
| `session/request_permission` | Sent when a capability needs approval; the client's answer becomes the approval. |
| `session/cancel` | Stops the conversation and answers the pending prompt with `cancelled`. |

Clients receive `agent_message_chunk` for text and `tool_call` /
`tool_call_update` for each capability and its result. Tool calls carry the devd
capability name, so `fs.write` in the client's timeline is the same call as
`fs.write` in Activity.

`Approval::Granted` is used only for an answer the client sent to
`session/request_permission`; every other call follows the ordinary path. A
client therefore cannot grant itself a capability devd would refuse a person.
`loadSession` is `false`: a client that needs history reads the devd conversation
by id. ACP-side MCP transports are off; devd's own tool servers are used through
its registry.

## Recreate

The `recreate` pack (`packs.install` with `builtin: "recreate"`) adds the
`recreator` role, the clean-room, study, three-width and ship skills, the
craft-anything skill and a disabled craft-anything tool server. An ACP client
such as craft-anything can then use `devd acp` as its builder.

See [`agents.md`](agents.md) for the team and [`runs.md`](runs.md) for evidence.
