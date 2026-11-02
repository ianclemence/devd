---
name: model-the-domain
description: Make the code's types and names match the concepts of the problem it solves
kind: design
roles: architect,implementer
---

Name things as the domain does. Make illegal states unrepresentable. One concept, one type, one owner. Do not let storage or transport shapes stand in for domain ideas.

The failure this prevents: strings and booleans where a small type would make misuse impossible.
