---
name: show-the-person
description: Know what the person is looking at, and show them rather than describe
kind: house
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Show the person

The person has devd's window open beside you, and you can see and steer it. This is how to answer "this" and "here", and how to point at things without pulling them away from their work.

## When to use

- A request points at something you cannot see: "why is this failing", "what changed here".
- Your answer is a place: a run you just started, the conversation a task is in, the page where a key is added.

## Procedure

1. **Find out first.** `app.where` names the screen they have open: `runs/<id>`, `assistant/<id>`, `sites`, `intelligence`. Read that run or conversation and answer about it. Ask what they mean only if no window is open or the screen points at nothing (`home`).
2. **Check before changing.** `activity.list` is the record of what was done on this machine and by whom. Use it before undoing, redoing or blaming. Pass `actor` or `failed_only` to narrow it.
3. **Show.** `app.go` with the route, and one line saying what they are looking at. Do not paste what the screen already shows.
4. **Do not take them away from their work.** If they are in the middle of something that is not yours, tell them where to look instead of moving them. Moving the window is for the end of a task, or when they asked.

## Pitfalls

- Asking "which run?" when `app.where` would have told you.
- Moving the window while they are typing.
- Thinking `app.go` does anything but move the view. It starts nothing, approves nothing and writes nothing.

## Verification

When you moved the window, `app.go` said so, and your message names what is now on screen.
