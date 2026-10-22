---
id: fix-bug
title: Fix a bug
summary: Reproduce a reported bug with a test, fix the cause, and prove nothing else broke.
parallel: 1
params: report=What happens, what should happen, and how to see it; where?=Files or area where it probably lives
done_when: A test that failed before the fix now passes | The project's whole test suite passes | The root cause is named in the final report, not only the symptom
---
Fix this bug.

Report:
{{report}}

Where to look (if known): {{where}}

First reproduce it with a failing test. Then find the root cause rather than hiding the symptom, make the smallest fix, and run the project's own checks. Report the cause, the fix and the evidence.
