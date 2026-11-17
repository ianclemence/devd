---
name: maestro
description: Write and run repeatable mobile UI flows from YAML
kind: check
roles: qa
---

# Maestro mobile flows

Maestro drives a real Android or iOS app through its accessibility tree, using a plain YAML flow. It turns a manual pass through a journey into a file that runs again on every change and in CI. It is an external CLI with its own prerequisites, not part of devd; this skill does not install it.

## When to use

- A mobile journey is worth checking on a device: sign-in, checkout, onboarding, a deep link.
- You need repeatable end-to-end evidence, not only unit tests or one screenshot.
- Not for a web page (`agent-browser`), a desktop app, or a single screen.

## Prerequisites

1. `maestro --version` with `project.exec`, and Java 17+ with `JAVA_HOME` set. Maestro needs both.
2. A device: `toolchain.devices` lists Android devices and emulators. On macOS, an iOS simulator needs Xcode; physical iOS is not supported.
3. The app's test build installed from the project's normal task. Use the test or staging build and its seed account, never a store build or the person's data. Ask before installing on a personal device.
4. If Maestro is missing, stop and ask the person to install the release archive and put its `bin` on `PATH`. Do not download or unpack it yourself, and do not change `PATH`.

## Procedure

1. Read the app id or URL from the project's configuration (`applicationId`, `android.package`, `ios.bundleIdentifier`, or the dev server address). Do not guess it.
2. Put the flow in `.maestro/<journey>.yaml` in the project. Start with `appId` or `url`, then a line of three hyphens, then the steps. One flow is one journey a person would describe.
3. Use visible text or accessibility ids with `tapOn`, `inputText` and `assertVisible`. Do not use coordinates; they break on the next device.
4. Assert the state after each change, not only at the end: the screen that should replace the old one, the error, the empty list.
5. Run `maestro test --test-output-dir=build/maestro-results .maestro/<journey>.yaml` with `project.exec` from the project.
6. Read the run through its artifacts, not the console alone: `commands.json` names the failed step, and `manifest.json` lists the screenshots, hierarchy and device log. A passing flow has no screenshot folder, so a failure screenshot is the proof.
7. For CI, run the same file with `--format junit --output build/maestro-report.xml`. Keep the flow in the repo and the output out of it.

## Pitfalls

- Testing the person's installed app or account. Use a build and data the project owns; ask before `clearState`.
- A fixed sleep. Maestro waits for the UI; sleep makes the suite slow and still flaky.
- Selectors tied to layout, a generated id, or a placeholder that changes with the copy.
- A flow that only walks the happy path. Add the denied permission, the empty state and the failed request where the journey allows it.
- Saying "tested on mobile" after one Android run when the flow is also meant for iOS or a tablet.
- Installing the CLI or editing `PATH` as part of a test run. That is a machine change for the person to make.

## Verification

The report names the flow file, device and command; says pass or fail; quotes the failing step and the line from `commands.json` or the device log; and names the artifact directory and the screenshot. It states what was not run, such as iOS, tablet, offline or a denied permission.
