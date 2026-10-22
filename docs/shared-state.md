# Shared scratchpads and todos

Scratchpads and todos are **project state**, not conversation history. They live
in the same SQLite store as sites, services and audit rows, and they survive
sessions, window reloads and application restarts. A person and an agent read and
write the same records through the same capabilities.

Both are scoped by the canonical absolute project path. Listing requires naming
the project. A todo or note in one project is not returned for another, even when
the names match.

## Scratchpads

A scratchpad is a named Markdown note: investigation results, a decision and its
reason, a constraint, a handoff. Names are unique within a project and may not
contain slashes or control characters.

| Capability | Tier | Purpose |
| --- | --- | --- |
| `scratch.list` | Read | Name, revision, author, timestamps, size, 240-character preview |
| `scratch.get` | Read | Full content and revision |
| `scratch.create` | Write | `{project,name,content?,task_id?}` |
| `scratch.update` | Write | `{project,name,content,expected_revision}` |
| `scratch.append` | Write | Add a section, creating the note if absent |
| `scratch.delete` | Destructive | Human approval required |

### Conflicts

Every write increments `revision`. `scratch.update` requires the revision the
caller read. A mismatch returns a conflict instead of overwriting: two agents
editing one note do not silently lose one another's work. The interface shows the
conflict and asks the person to reload and merge.

`scratch.append` reads the current revision and appends in one step, which is the
right operation for an agent recording a finding; a simultaneous human edit still
produces a conflict rather than a merge.

Writes record the author (`human:http`, `agent:claude-code`, …) and update the
timestamp. There is no hidden authorship.

## Todos

| Field | Meaning |
| --- | --- |
| `id` | Stable UUID |
| `project` | Canonical project path |
| `task_id` | Optional association with a team task |
| `title`, `description` | What needs doing |
| `status` | `open`, `in_progress`, `blocked`, `done`, `cancelled` |
| `priority` | `low`, `normal`, `high`, `urgent` |
| `assignee` | Optional person or agent label |
| `origin` | Who created it |
| `dependencies` | Optional todo ids in the same installation |
| `revision` | Optimistic concurrency |
| `created_at`, `updated_at`, `completed_at` | Timestamps |

| Capability | Tier | Purpose |
| --- | --- | --- |
| `todo.list` | Read | Filter by task and status, ordered by priority then age |
| `todo.get` | Read | One todo |
| `todo.create` | Write | Title, description, priority, assignee, task scope |
| `todo.update` | Write | Partial update with `expected_revision` |
| `todo.complete` | Write | Move to `done` |
| `todo.reopen` | Write | Move back to `open` |
| `todo.note` | Write | Add history without changing status |
| `todo.history` | Read | Recorded transitions and notes |
| `todo.delete` | Destructive | Human approval required |

Status transitions are appended to `todo_events`; prose notes are not retained as
conversation. A todo's current value plus its transition history is enough to
understand how it got there without storing an agent transcript in the task
table.

## Agent handoff

A coordinating agent can:

1. `scratch.append` its findings to a project note.
2. `todo.create` an actionable item assigned to another agent, with the project
   and optional task scope.
3. `team.assign` the work to another member of the team with instructions that name the note and todo.
4. Follow the run with `team.get` without blocking the interface.
5. Update the todo when the work is accepted.

The second agent reads the same records. Nothing is passed through an
unrestricted shared text file, and the note is not automatically visible outside
its project.

## Visibility and limits

* There is no encryption at rest in SQLite; the same local user who can read
  `state.db` can read the notes. This is a single-user development tool, not a
  multi-tenant service.
* An agent can list the notes in a project it knows the path of. It is not
  granted access to another project's notes by default, and no capability
  enumerates every project's shared state.
* Deleting a note or todo requires human approval because it destroys data.
* Note content is not size-capped by the schema. The interface is intended for
  working notes; large binary data belongs elsewhere.
* A project path that no longer exists cannot be canonicalised, so its shared
  state is not reachable through the capability API until the directory returns.
  The rows remain in the database.
