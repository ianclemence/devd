---
id: security-review
title: Security review
summary: Read the project for vulnerabilities and report findings without changing code.
parallel: 1
params: scope?=A folder, feature or area to focus on (the whole project when empty)
done_when: Findings are listed with severity, location and a suggested fix | No code was changed
---
Review this project for security problems.

Focus: {{scope}}

Look for injection, broken authentication and authorisation, unsafe handling of secrets, unsafe file and path handling, dependency risks and anything that trusts input it should not. Read the code and run the project's own checks; do not change the code. Report each finding with severity, where it is, why it matters and how to fix it, and say plainly what you did not check.
