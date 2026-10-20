# `devd.yml`

A project can declare the processes it needs in a versioned `devd.yml` at its
root. devd validates the file, shows you the exact commands, and **runs nothing
until a human approves that exact manifest**. Editing any command invalidates the
approval.

## Example

```yaml
version: 1

project:
  name: example-app
  env:
    APP_ENV: development

processes:
  web:
    command: ["npm", "run", "dev"]
    working_dir: "."
    env:
      PORT: "3000"
    health:
      url: "http://127.0.0.1:3000/"
      timeout_secs: 60
      interval_ms: 500
    restart: on-failure

  worker:
    command: ["npm", "run", "worker"]
    working_dir: "."
    depends_on: ["web"]
    startup_timeout_secs: 20

  assets:
    command: ["npm", "run", "watch"]
    working_dir: "."
    restart: never
```

## Schema, version 1

Top level:

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `version` | integer | yes | Must be `1`. |
| `project.name` | string | yes | Display name, 1–80 characters. |
| `project.env` | map | no | Environment for every process. |
| `processes` | map | yes | 1–32 named processes. |

Each process:

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `command` | **array of strings** | required | Executable followed by arguments. A string is rejected. |
| `working_dir` | relative path | `.` | Must resolve inside the project root; `..` and absolute paths are rejected. |
| `env` | map | `{}` | Variables for this process, merged over `project.env`. |
| `health.url` | loopback URL | — | HTTP check; 2xx/3xx is healthy. Exactly one of `url` or `port`. |
| `health.port` | integer | — | TCP loopback check. |
| `health.timeout_secs` | integer | `30` | How long readiness may take. |
| `health.interval_ms` | integer | `500` | Poll interval, clamped to 50–30000. |
| `restart` | `never`, `on-failure`, `always` | `on-failure` | Supervisor policy, at most 5 restarts. |
| `depends_on` | array of names | `[]` | Start order; cycles and unknown names are rejected. |
| `startup_timeout_secs` | integer | `30` | Readiness window, 1–600. |

Unknown keys are rejected, so a typo is a validation error rather than a setting
that silently does nothing.

## What validation enforces

* `command` is an argv array. It is never joined into a shell string.
* Control characters and NUL bytes are refused. Arguments are capped at 4 KiB
  each and 32 KiB in total.
* `working_dir` may not be absolute and may not contain `..`; the canonical path
  must still be inside the canonical project root, which also defeats a symlink
  escape.
* Health URLs must parse, use `http`/`https`, and resolve to a loopback host
  (`localhost`, `127.0.0.0/8`, `::1`). A manifest cannot make devd probe a remote
  host.
* Environment keys must be valid names. `DEVD_*` is reserved. Values are capped
  at 8 KiB.
* Dependency cycles, self-dependencies and missing dependencies are refused.
* The file is capped at 256 KiB.

## Environment

A process sees an explicit baseline needed to find tools and temporary
directories (`PATH`, `PATHEXT`, `SYSTEMROOT`, `TEMP`, `HOME`, `USERPROFILE`,
`APPDATA`, `LOCALAPPDATA`, `PROGRAMFILES`, …), then `project.env`, then the
process's `env`.

It does **not** inherit the rest of the devd process's environment. Provider API
keys and other credentials in the daemon's environment are not silently handed to
a project. If a process needs a secret, declare it explicitly.

Values of variables whose names look secret (`TOKEN`, `SECRET`, `PASSWORD`,
`API_KEY`, `PRIVATE`, `CREDENTIAL`, `AUTH`, …) are redacted from `project.logs`
output when they appear. This is a best-effort log scrubber, not a guarantee that
a program cannot print a secret in another form.

## Approval

`project.plan` parses the file and returns:

* the semantic SHA-256 of the manifest (comments and whitespace do not change it),
* every command and working directory,
* environment **keys** with secret values masked,
* health, restart and dependency declarations,
* whether the current hash is approved.

`project.approve` records the hash against the project path. It is `human_only`:
an agent can read the plan, but it cannot approve executable configuration.
`project.start` and `project.restart` refuse without a matching approval. A change
to any command, environment value, health check, restart policy or dependency
invalidates the approval.

## Supervision

* Each process is spawned with an argv array, an explicit environment, a working
  directory and a log file under `DEVD_HOME/projects/<slug>/logs/`.
* `Start all` walks dependencies in order and waits for each process to become
  ready before starting its dependants. A readiness failure is reported with the
  last log line.
* `on-failure` restarts a non-zero exit; `always` restarts any exit; `never` does
  not restart. At most five restarts.
* A manifest edited after approval does not restart a process under its new
  commands; it is marked failed with an explanation until re-approved.
* `Stop` and `Restart` terminate the **whole process tree** (`taskkill /T /F` on
  Windows, a process group on Linux).
* Status, pid, exit code, restart count, health and the last failure are durable
  in SQLite. The UI polls them; a view change does not lose the supervisor.
* A project process is owned by the devd process that started it. A second devd
  process reports it as supervised elsewhere and refuses to stop or duplicate it.
  On startup, a row whose owner is gone is reconciled to failed with an honest
  message rather than reported as running.
* Devd shutdown stops every process it owns gracefully.

## Platform notes

* On Windows, npm and similar tools install `.cmd` shims rather than `.exe`
  files. The supervisor starts those through `cmd.exe /c <shim> <args>`. The
  arguments remain separate argv entries; no shell string is constructed.
* On Linux, process-group termination uses `kill(-pid)`. A service that daemonises
  and leaves the process group cannot be tracked by this model.
* If the devd process is killed with `TerminateProcess` on Windows, children may
  outlive it. The next start reports them as unsupervised; Windows Job Objects
  with kill-on-close are the known fix and are not implemented.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/api/project?project=` | Validated plan and approval state |
| `POST` | `/api/project/approve` | `{project}` |
| `GET` | `/api/project/status?project=` | Declared processes with live state |
| `POST` | `/api/project/start` | `{project,name,all}` |
| `POST` | `/api/project/stop` | `{project,name,all}` |
| `POST` | `/api/project/restart` | `{project,name}` |
| `GET` | `/api/project/logs?project=&name=&lines=&query=` | Redacted tail |

The equivalent capabilities are `project.plan`, `project.approve`,
`project.status`, `project.start`, `project.stop`, `project.restart` and
`project.logs`.
