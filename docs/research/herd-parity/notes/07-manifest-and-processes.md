# 07 — Project manifest and processes (C01–C04, D07)

**Evidence.** `devd.yml` (`docs/devd-yml.md`, `crates/devd-workspace/src/manifest.rs`, 744 lines): argv-only commands, working directory confined to the project, loopback health checks, explicit environment, approval bound to the exact manifest content; editing invalidates approval. Herd's `herd.yml` holds name, php, secured, aliases, services and integrations. They are not equivalent: one describes processes, the other project identity.

**Finding.** The safest design is to extend `devd.yml` with identity fields rather than add a second file. DEVD's approval model is stronger. Herd's auto-detection of dev, vite, pail and queue processes is better UX (D07); DEVD derives tasks from stack definitions instead.

**Tests to add.** A manifest with `runtime` and `services` validates and reports one message per problem; approval is invalidated by identity-field edits.
