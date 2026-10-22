---
name: product-requirements
description: Write a one-page requirements document an engineer can build from without asking what was meant.
kind: playbook
roles: product,planner
---

# Product requirements

Use when an idea has to become buildable work.

1. **Problem.** Who has it, what they do today, and why that is not good enough. One paragraph, no solution in it.
2. **Scope.** What the first useful version does, and a separate list of what it explicitly does not. Anything not needed for a first version moves to the second list.
3. **Stories.** `As <who>, I can <do what>, so that <why>`, each with acceptance criteria a person or a test can answer yes or no to.
4. **Constraints.** Platforms, data that must not be lost, anything that must keep working as it does.
5. **Open questions.** Each with who can answer it and what changes depending on the answer.
6. **Order.** The sequence to build in, with the reason for it.

Mark every assumption as an assumption. Do not invent users, numbers or dates. A requirement that cannot be checked is not finished: rewrite it until it can.
