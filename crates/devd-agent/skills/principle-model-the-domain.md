---
name: principle-model-the-domain
description: Make the code's types and names match the problem's concepts.
kind: principle
roles: architect,implementer
---

# Make the code's types and names match the problem's concepts

Name things as the domain does. Make illegal states unrepresentable. One concept, one type, one owner. Do not let storage or transport shapes stand in for domain ideas.
Smell: strings and booleans where a small type would make misuse impossible.
