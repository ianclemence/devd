---
name: ui-design-review
description: Check a screen for hierarchy, states, spacing, copy and consistency with the existing interface.
kind: technique
roles: designer,frontend,reviewer
---

# UI design review

Go through a screen in this order and fix or report each failure.

1. **One job.** What is this screen for? If you cannot say in a sentence, it is doing two jobs.
2. **Hierarchy.** The most important thing is the most prominent; the primary action is obvious and there is one of it.
3. **States.** Empty (says what to do next), loading, error (says what went wrong and what to try), success, and no permission.
4. **Widths.** Check narrow and wide. Nothing overflows horizontally; touch targets stay large enough to hit.
5. **Consistency.** Spacing, type sizes, colours and component shapes come from the existing tokens and components. A new value needs a reason.
6. **Copy.** Plain words, the user's vocabulary, verbs on buttons ("Save changes", not "OK"). No text that explains a confusing interface instead of fixing it.
7. **Feedback.** Every action visibly does something, quickly; slow work shows progress attached to what it is for.

Report what you checked and how. Say what you could not see.
