---
name: prototype
description: Build a throwaway sketch to settle one design question, then discard it
kind: build
roles: implementer,architect
---

# Prototype

Answers one design question by building the least that can answer it. The prototype is thrown away; only the answer is kept.

## When to use

A decision that is cheaper to try than to argue: will this approach perform, fit, or feel right.

## Procedure

1. Write down the one question and what result would decide it.
2. Build the least that can answer it. No tests, no polish, no error handling beyond what the question needs.
3. Run it and record the result.
4. Report the answer and the evidence. Delete the prototype, or keep it clearly labelled and outside the product.
5. Implement the decision properly, from scratch.

## Pitfalls

- A prototype that becomes the implementation by drift.
- Answering several questions at once, so none is settled.

## Verification

The report states the question, the result and the decision, and the prototype is gone or labelled.
