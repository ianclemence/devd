# Writing a built-in skill

A skill is a file an agent reads in full when it starts a kind of work. Every skill's one-line description is in every prompt; its body is read only when it is called. These rules keep the first cheap and the second worth reading. A test (`every_built_in_skill_meets_the_authoring_standard` in `src/skills.rs`) enforces the ones that can be checked.

The format and the standards below follow what works in agent skills generally: short descriptions, a fixed section order, native tool names, helper scripts rather than inline code, explicit boundaries and fallbacks, the exact failure state, references loaded on demand, and the Agent Skills convention (a name, a description that says when to use it, a body that explains why).

## File

`skills/<name>.md`, starting with frontmatter:

```
---
name: <lowercase-words-with-hyphens>
description: <what it does, one line, 90 characters at most, no final full stop>
kind: discover | design | build | check | review | ship | words | house
roles: <comma-separated roles that are offered it>
---
```

Add the file to `src/skills_data.rs` (alphabetical). A skill that needs more than 100 lines is two skills, or has a reference file it points to.

## Description

It is the index line, read on every turn, so it is the cheapest text in the product and the most read.

- Say what it does, as a verb phrase. Include the words a person would say ("review", "commit", "migration") so the right skill is found.
- 90 characters at most. No marketing words. Do not repeat the name.
- A boundary beats a boast. "Not for X, use Y" belongs in the body's *When to use*.

## Body

In this order, each short:

1. `# Title`, then two or three sentences: what it does, and what it does not.
2. `## When to use`: the situations, and the nearest skill for what it is not.
3. `## Prerequisites` only if something must exist first (a tool, a running service) and how to check.
4. `## Procedure`: numbered, imperative, each step something you can do or check.
5. `## Pitfalls`: the failures that actually happen, each with what to do instead.
6. `## Verification`: what proves it is done. Evidence an agent can produce, not a feeling.

Explain why where a rule is not obvious; a model that knows the reason applies it to the case you did not foresee. Do not write rules in capitals.

## Tools

Name the devd capability or tool that does the thing: `search_files`, `read_file`, `edit_file`, `project.exec`, `browser.snapshot`, `git.diff`. Do not name the shell utility it replaces (`grep`, `cat`, `sed`, `find`): the capability is checked, recorded and permitted; the utility is none of those.

## Reference skills by name

A backticked hyphenated word is a link to another skill and must exist. The tests fail on a dangling one.

## What does not belong

A tour of what the tool is. Advice that fits any project. A rule that a type, a lint or a test could enforce (put it there: `plant-the-fix-in-the-structure`). Anything that will be wrong in a month.
