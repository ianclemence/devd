---
name: cut-a-release
description: Prepare a release: version, changelog, checks on the final commit, and a way back
kind: ship
roles: release,implementer,infra
---

# Cut a release

Turns finished, reviewed work into a release someone can install, with a record of what is in it and a way back. It prepares everything and stops before anything public: pushing, tagging a public release, publishing and deploying are the person's decisions.

## When to use

"Release", "cut a version", "ship 1.4", or when a change is ready to go out.

## Procedure

1. **Follow the project's process.** Read its release notes or docs, its CI configuration and the last tags in `git.log`.
2. **What is going out.** The commits since the last release, grouped by what a user would notice, plus any migration, configuration change or warning.
3. **The version.** By the project's own rule (fixes, features, breaking changes), with the reason.
4. **Checks on the exact commit.** Test, lint, typecheck and build on the commit that will be released, not an earlier one.
5. **Artefacts.** Build them as CI does. Install them, start them, and check the version they report is the one you chose.
6. **Changelog** for the readers: what changed for them, in plain words, in the project's style (`write-the-commit`, `plain-speech`).
7. **The way back.** The previous version's artefacts and the exact step to return to them.

## Pitfalls

- Skipping a hook or a failing check to get out. A red check is information.
- A changelog that lists commits instead of changes.
- Tagging before the artefacts are verified.

## Verification

Report the version and why, what is in it, the checks on the final commit and results, the artefacts and how you verified them, the way back, and the exact steps left for a person.
