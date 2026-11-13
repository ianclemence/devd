---
name: create-verification-skill
description: Write a project-specific, repeatable way to prove the project works
kind: build
roles: planner,implementer,qa
---

# Create a verification skill

Most projects have a way to prove they work that nobody wrote down. This writes it once, as a skill, so every agent uses the same proof.

## When to use

A project with no written verification, or one where agents keep verifying differently.

## Procedure

1. Find it: `project.detect` lists the tasks; read the README, CI configuration and Makefile for what the maintainers run before merging.
2. Write a short skill with `skills.save`, named `verify-<project>`, listing the exact `project.run` tasks in order, expected results, how long they take, what needs running services, and how to use the main behaviour by hand.
3. Run it yourself end to end and fix it until a cold reader could follow it.
4. Update it in the same change whenever verification changes.

## Pitfalls

- Copying CI configuration without running it locally.
- Steps that only work on your machine.

## Verification

You ran the finished skill from the top and it passed, and its run time is written in it.
