---
name: ui-design-review
description: Check a screen for hierarchy, states, spacing, copy and consistency
kind: review
roles: designer,frontend,reviewer
---

# UI design review

Goes through a screen in a fixed order and fixes or reports each failure.

## When to use

A new or changed screen, before it is called done. For keyboard and screen-reader use see `accessibility-check`.

## Procedure

1. **One job.** What is this screen for? If you cannot say in a sentence, it is doing two jobs.
2. **Hierarchy.** The most important thing is the most prominent; the primary action is obvious and there is one of it.
3. **States.** Empty (says what to do next), loading, error (what went wrong and what to try), success, no permission.
4. **Widths.** Narrow and wide. Nothing overflows; touch targets stay large enough.
5. **Consistency.** Spacing, type sizes, colours and component shapes come from existing tokens and components. A new value needs a reason.
6. **Copy.** Plain words, the user's vocabulary, verbs on buttons ("Save changes", not "OK"). No text that explains a confusing interface instead of fixing it.
7. **Feedback.** Every action visibly does something, quickly; slow work shows progress attached to what it is for.

## Pitfalls

- Reviewing a mock or the code instead of the rendered screen (`agent-browser`).
- A new colour or size with no reason.

## Verification

The report says what you checked and how, and what you could not see.
