---
id: release-prep
title: Prepare a release
summary: Bring version, changelog, docs and checks in line for a release, without publishing it.
parallel: 1
params: version=The version number to prepare
done_when: The version is updated everywhere it appears | The changelog describes what changed since the last release | The test suite and build pass | Nothing was published or pushed
---
Prepare release {{version}}.

Update the version wherever the project records it, write the changelog entry from the history since the last release, check the documentation still matches, and run the project's tests and build. Do not tag, push or publish anything; report what is ready and what a person must still do.
