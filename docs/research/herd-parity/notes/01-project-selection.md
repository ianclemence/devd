# 01 — Project selection and detection (P01, P02, P06, P07, P14)

**Workflow.** Herd: prior REA ASAR/preload read (`sites.addExistingSite`, no args, returns a path) plus screen capture of the native dialog. DEVD: source read and test lookup. No new Herd runtime action this session.

**Evidence chain.**
- Herd: "Link Existing App" → IPC `sites.add-existing-site` → OS folder dialog titled "Select the folder of your existing PHP project." (`../../01-herd-investigation.md` §6).
- DEVD desktop: button → `POST /api/pick-folder` (`crates/devd-server/src/api.rs:1120`) → `rfd` in `crates/devd-server/src/picker.rs`. Web: `fs.browse` (`crates/devd-core/src/browse.rs`, 616 lines) with places, roots, breadcrumb and symlink marking. The highlighted folder is inspected via `project.detect` before import (`crates/devd-stacks/src/detect.rs`).
- The same picker is reused in Projects, Develop, the agent working directory and the clone destination (CHANGELOG 0.9.0).

**Verified.** Manual path entry is not the only way to choose a project: a native picker exists (desktop) and a browser fallback (web). Contract test: `crates/devd-server/tests/ui_contract.rs::the_folder_picker_is_the_normal_way_to_choose_a_project`. The full workspace suite was green this session.

**Alternatives considered.** The picker might exist but not feed detection. Refuted by the CHANGELOG and the Develop flow in `app.js`; not re-run end to end this session.

**Uncertain.** macOS behaviour (U-15). Herd's `edit` (open in IDE) has no DEVD counterpart (P07).

**Regression tests to add.** `pick-folder` cancel has no side effects; browsing refuses paths outside permitted roots if such a rule exists (read `browse.rs` tests first).
