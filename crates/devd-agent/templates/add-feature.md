---
id: add-feature
title: Add a feature
summary: Design, build, test and document a feature in an existing project.
parallel: 2
params: feature=What the feature should do and for whom
done_when: The feature works as described and is covered by tests that pass | The existing tests still pass | The README or docs describe how to use it
---
Add this feature to the project.

Feature:
{{feature}}

Start with the product view (who it is for, what done looks like), then the design that fits the project's structure, then the build with tests, then an independent review. Keep the change as small as the feature allows.
