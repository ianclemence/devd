---
name: product-requirements
description: Write a one-page requirements document an engineer can build from
kind: design
roles: product,planner
---

# Product requirements

Turns an idea into buildable work: a page an engineer can build from without asking what was meant. It invents no users, numbers or dates.

## When to use

An idea, request or complaint that has to become work.

## Procedure

1. **Problem.** Who has it, what they do today, why that is not good enough. One paragraph, no solution in it.
2. **Scope.** What the first useful version does, and a separate list of what it does not.
3. **Stories** (`user-stories`). `As <who>, I can <do what>, so that <why>`, each with acceptance criteria that can be answered yes or no.
4. **Constraints.** Platforms, data that must not be lost, anything that must keep working as it does.
5. **Open questions.** Each with who can answer it and what changes with the answer.
6. **Order.** The sequence to build in, with the reason.

## Pitfalls

- A requirement that cannot be checked. Rewrite it until it can.
- Unmarked assumptions. Mark every one.

## Verification

Each story has a criterion a person or a test can answer, and each open question has an owner.
