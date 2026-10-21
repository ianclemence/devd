# 03 — Runtimes and toolchains (R01–R08)

**Workflow.** Herd: `php:*`, `node:*`, `use`, `isolate`, `which-php`, `ini` from the CLI and `php.json`. DEVD: `runtime.*` capabilities, `catalogue.rs`, `site.set_php`, `tools.yaml`.

**Evidence.** `catalogue::php` returns an empty list when `!cfg!(windows)` (`crates/devd-runtime/src/catalogue.rs:104`). Node and Go publish Linux tarballs and are installed. `site.set_php` is implemented, including a plan that names the installed versions (`crates/devd-sites/src/caps.rs:127`). DEVD installs three runtimes itself; the rest come through `toolchain.*` (apt, dnf, pacman, brew, winget).

**Finding.** `../02-feature-matrix.md` lists per-site PHP routing as *Partial*; the code and `limitations.md` say it is done. The matrix row is stale (R03).

**Verified.** PHP, Node and Go install and verify on Windows (tests depend on the network). Runtime reporting is consistent across surfaces (commit f48c519).

**Uncertain.** Whether commands run through `project.exec` get the pinned runtime first on PATH (U-08). There is no patch-update flow comparable to Herd's `installed_internal_8.5` counter.

**Tests to add.** PATH construction per project runtime; `runtime.update` idempotency once built.
