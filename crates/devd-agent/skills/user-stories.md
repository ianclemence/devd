---
name: user-stories
description: Split work into small stories with acceptance criteria that read as checks
kind: design
roles: product,planner
---

# User stories

A story is the smallest piece of work that changes what a person can do.

## When to use

Breaking a requirement into buildable pieces (`product-requirements`).

## Procedure

1. Name the person (a role, not "the user"), the action and the reason.
2. Write two to five acceptance criteria as observable outcomes: "after saving, the list shows the new name", not "saving works".
3. Include the unhappy path: empty input, no permission, no network, a duplicate.
4. If a story needs more than a few days, or two unrelated criteria, split it. If two stories cannot be finished separately, they are one.
5. Order them so each one leaves the product usable.

## Pitfalls

- A criterion nobody can check.
- A story that is a task ("add a column").

## Verification

Each criterion can be answered yes or no by a person or a test.
