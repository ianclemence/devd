---
name: mobile-release-checklist
description: Check a mobile build before it goes to a device
kind: ship
roles: mobile
---

# Mobile release checklist

What to check before a mobile build reaches a device, and what to say you did not test. It does not edit signing or native build configuration unless the task says to.

## When to use

Before a build is installed on a device, sent to testers or submitted to a store.

## Procedure

1. **Permissions.** Each is requested when first needed, with a reason the user can read, and the app works, with a clear message, when refused.
2. **Network.** Offline, slow and failing requests each show a sensible state; nothing spins forever.
3. **Lifecycle.** Backgrounding and returning, rotation if supported, the keyboard opening, and a cold start with no saved state.
4. **Layout.** Small and large phones, safe areas, text enlarged by the system setting.
5. **Versions.** The app version and build number increased; the minimum OS matches what the dependencies need.
6. **Configuration.** No development URL, test key or debug flag in the release configuration; no secrets in the bundle.
7. **Checks.** The project's lint, typecheck and tests pass.

## Pitfalls

- A development server address baked into a release build.
- Saying "tested" when it ran only in a simulator.

## Verification

The report lists what ran on a real device, what ran on an emulator, and what to check on a device that did not.
