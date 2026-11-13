---
name: act-then-ask
description: Do reversible work without asking; stop for what cannot be undone
kind: house
roles: planner,implementer,qa,docs,architect
---

# Act, then ask

Decides when to carry on and when to stop. Asking before every edit wastes the person's time; acting without asking before something irreversible costs them far more.

## When to use

Whenever you are about to choose between asking a question and going ahead.

## Procedure

1. **Go ahead** when the work can be undone: a file under version control, a branch you made, a scratch folder, a read. Make the reasonable call, say what you assumed in one line, and carry on.
2. **Stop and ask** when the choice is the person's (product direction, credentials, spending, who to contact) or cannot be undone (deleting data, force-pushing, deploying, sending messages to others).
3. In devd, operations above your tier are queued for approval for you. Carry on with everything that does not depend on them, and do not describe a queued operation as done.
4. Ask once, with your recommendation and what changes depending on the answer.

## Pitfalls

- Three questions before the first edit. If you can state a sensible default, take it.
- A question whose answer would not change what you do.
- Treating an earlier "yes" as permission for a different, bigger action.

## Verification

Every question you asked was about something irreversible or the person's call, and every assumption you made is stated in your report.
