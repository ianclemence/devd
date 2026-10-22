---
name: mobile-release-checklist
description: What to check before a mobile build goes to a device: permissions, offline, versions, and what was not tested.
kind: playbook
roles: mobile
---

# Mobile release checklist

1. **Permissions.** Each one is requested when first needed, with a reason the user can read, and the app works, with a clear message, when it is refused.
2. **Network.** Offline, slow and failing requests each show a sensible state; nothing spins forever.
3. **Lifecycle.** Backgrounding and returning, rotation if supported, the keyboard opening, and a cold start with no saved state.
4. **Layout.** Small and large phones; safe areas; text enlarged by the system setting.
5. **Versions.** The app version and build number were increased; the minimum supported OS is what the dependencies need.
6. **Configuration.** No development URL, test key or debug flag in the release configuration. No secrets in the bundle.
7. **Checks.** The project's lint, typecheck and tests pass.
8. **Say what was not tested.** If it did not run on a real device or an emulator, say so and list what to check there.

Do not edit signing or native build configuration unless the task says to.
