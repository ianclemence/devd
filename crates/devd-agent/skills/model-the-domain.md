---
name: model-the-domain
description: Make types and names match the concepts of the problem
kind: design
roles: architect,implementer
---

# Model the domain

Names things as the domain does and makes illegal states unrepresentable, so misuse does not compile.

## When to use

Designing new types, or when strings and booleans stand in for concepts.

## Procedure

1. List the concepts as the people who use the thing name them.
2. One concept, one type, one owner (`decide-who-owns-the-state`).
3. Make illegal states unrepresentable (`put-rules-in-the-compiler`).
4. Do not let storage or transport shapes stand in for domain ideas.

## Pitfalls

- `status: string` with five valid values.
- A boolean pair that allows an impossible combination.

## Verification

A wrong state cannot be written in the new types, and the names read like the domain's own words.
