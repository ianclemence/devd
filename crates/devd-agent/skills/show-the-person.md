---
name: show-the-person
description: Know what the person is looking at before you answer "this" or "here", and take their window to the thing you mean, without pulling them away from their work
kind: house
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

The person has devd's window open beside you, and you can see and steer it.

**"This", "here", "that one".** When the request points at something you cannot see ("why is this failing?", "what changed here?"), call `app.where` before you ask. It names the screen they have open: `runs/<id>`, `assistant/<conversation id>`, `sites`, `intelligence`. Read that run or that conversation, and answer about it. Ask what they mean only if `app.where` says no window is open or the screen is one that points at nothing (`home`).

**Show, do not describe.** When your answer is a place (the run you just started, the conversation a task is in, the page where a key is added, the activity record of what you changed), call `app.go` with that route and say in one line what they are looking at. Do not paste what the screen already shows.

**Do not take them away from their work.** Call `app.where` first. If they are in the middle of something that is not yours (a conversation they are typing in, a run they are reading), tell them where to look instead of moving them. Moving the window is for the end of a task, or when they asked to be shown.

**Check what happened before you change it.** `activity.list` is the record of what has been done on this machine and by whom, newest first. Use it before undoing, redoing or blaming: it says who ran what, through which surface, and what the outcome was. Pass `actor` (`human`, `agent`, `system`) or `failed_only` to narrow it.

What `app.go` does not do: it changes only what the window shows. It starts nothing, approves nothing and writes nothing. For any of those, use the capability that does, through the usual approvals.
