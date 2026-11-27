---
name: device-testing
description: Drive a mobile app on an emulator or phone and prove the changed flow works
kind: check
roles: mobile,qa,implementer
---

# Device testing

A mobile change is not proven until the flow has been run on a device. The Argent tool server (the `mcp__argent__`
tools) drives Android emulators and phones, iOS simulators on a Mac, and Chromium apps.

## When to use

Any change a person would see or touch in a mobile app: a screen, a gesture, navigation, a form, a permission, a
crash or a performance complaint.

## Before starting

If you have no tool whose name starts with `mcp__argent__`, say so in your report: a person adds it by installing the
`mobile` pack and turning the `argent` server on in Team > MCP. Then prove what you can with the project's checks and
list what still needs a device. Never claim a screen works because it type-checks.

## Procedure

1. list-devices; use a running one, or boot-device. On Windows and Linux only Android is available.
2. Build and install with the project's own commands, then launch-app. For React Native keep one Metro running.
3. describe before every tap: act on what the screen says is there, not on remembered coordinates.
4. Walk the changed flow with gesture-tap, gesture-swipe, keyboard and button, waiting with
   await-ui-element or await-screen-idle instead of pausing.
5. Try what breaks phones: the keyboard open over a field, a long name, no network (network-conditions), a denied
   permission, rotation when the app allows it, and the back button.
6. Capture proof with `screenshot`; compare against a baseline with screenshot-diff.
7. For errors read debugger-log-registry and view-network-logs before guessing.

## Verification

- A screenshot of the final state of each flow you claim works, and the device and OS it ran on.
- Logs clean, or each error explained.

## Pitfalls

- Reporting a flow as working from a build that succeeded.
- Reinstalling or wiping a person's device or data without being asked.
- Typing real credentials or personal data into an app under test.
- Leaving recordings, servers or emulators running that you started.
