---
name: create-verification-skill
description: Write a project-specific, repeatable way to prove the project works.
kind: technique
roles: planner,implementer,qa
---

# Create a verification skill for this project

Most projects have a way to prove they work that nobody wrote down. Write it once so every agent uses it.

1. Find it: `project.detect` lists the tasks; read the README, CI configuration and Makefile for what the maintainers run before merging.
2. Write a short skill (`skills.save`, user skills) named `verify-<project>` with the exact `project.run` tasks in order, expected results, how long they take, what needs running services, and how to exercise the main behaviour by hand.
3. Run it yourself end to end and fix the skill until a cold reader can follow it.
4. Keep it current: when verification changes, update the skill in the same change.
