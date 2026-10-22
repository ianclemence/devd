---
id: spec-to-pr
title: Spec to reviewed change
summary: Take a written specification to a tested, reviewed change in the project.
parallel: 2
params: spec=The requirements, ticket or user story to build
done_when: Automated tests cover the behaviour in the specification and pass | The change was reviewed and no blocking issue is open | Anything a user or developer would notice is documented
---
Build what this specification asks for, in this project.

Specification:
{{spec}}

Plan the smallest change that satisfies it. Read the code before changing it, follow the project's own conventions, and add tests for the behaviour the specification names. Have the change reviewed by someone who did not write it, and say in the final report what changed, what was verified and anything left open.
