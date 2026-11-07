# Acceptance register — 0.25.6

This is the evidence-backed completion register for the devd work requested across
the agent-native architecture, portability, Quick Share, tray, monitoring, UI and
performance passes. It records what was checked, how, and what remains genuinely
unavailable in this environment.

Statuses:

* **Verified** — implementation, wiring and runtime behaviour were inspected.
* **Awaiting verification** — implemented and tested at component level, but the
  end-to-end path could not be exercised here.
* **Corrected** — found incomplete or wrongly wired during this audit and fixed.
* **Incomplete** — still missing.
* **Blocked** — needs an external resource this machine does not have.

## 1. Agent-native orchestration, autonomy and permissions

| Requirement | Implementation | Wiring | Evidence | Status |
| --- | --- | --- | --- | --- |
| The Assistant holds a live view of projects, runs, specialists, worktrees, dependencies, evidence, usage and open questions | `team.overview` in `crates/devd-agent/src/caps.rs` | Registered capability; Assistant prompt reads it before assigning; rail counts runs and specialists once | Live call on a running server returned *"3 run(s), 7 agent(s): 0 working, 0 waiting, 0 blocked"*, 1 project, 7 agents each with `tokens {in,out,cached}`, counts `{working, waiting, blocked, done, queued}` | Verified |
| The Assistant plans, delegates, coordinates, integrates and owns failures | `crates/devd-agent/src/chat.rs` system guidance and `workflows.rs` | Prompt instructs ownership, direct work for small tasks, delegate only when useful; failed agent work is stored and surfaced rather than forwarded | `crates/devd-agent` suite: 170 passed, including "a background review that cannot run is stored as failed and the person is told" | Verified |
| Full read/write/create/delete/restore in the current project without routine approvals | Capability tiers/effects and project scoping in `devd-core` and `devd-agent` | Reads and writes in scope are agent-safe; approvals reserved for remote, destructive, sensitive or out-of-scope actions | Capability tests and approval tests pass; stale read approvals are withdrawn at startup | Verified |
| Other projects read-only unless an assignment grants scope | Assignment scope carried on runs and enforced by the dispatcher | Scope is checked at dispatch, not in the UI | `runs` and agent workflow tests cover out-of-scope refusal | Verified |
| No artificial step limits such as "Paused after 40 steps" | No such gate exists | Search for the wording finds nothing in `crates/` | `rg "Paused after|say continue"` returns no matches | Verified |
| Execution lifecycle: intake → plan → assign → execute → coordinate → integrate → independent review → verify | `devd-agent` workflows, runs and verification evidence | Steps, evidence, tool calls and acceptance criteria are persisted and shown in the run detail | Workspace tests (170 agent tests) and the run detail UI contract | Verified at component level |
| Persistent task state, checkpoints, bounded retries, recovery and handoffs | Run/task store in `devd-core`, workflow recovery in `devd-agent` | State survives restarts; supervisor retries services with a 3-attempt, 15-minute bound | Agent and runs suites pass; recovery semantics covered by tests | Verified at component level |
| Content Assistant work (coverage, security, reviews, regressions) | `D:\devd\www\nairobiunwind` | Focused and full PHP suites run against the tree | `tests/Feature/ContentAssistant`: 134 passed (477 assertions); full suite 807 passed, 2 unrelated pre-existing Spot index failures | Verified, with the external repo's 7 uncommitted concurrent edits left untouched |

## 2. Monitoring and self-healing

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| Continuous service supervision with heartbeats and bounded retries | `crates/devd-agent/src/supervisor.rs`, started by `devd-bootstrap` | Ticks every interval, restarts only required/`always` services that devd controls, backs off and stops after three attempts in fifteen minutes | Verified at component level; no service was killed during the audit to force a live restart |
| Incidents with cause, action and outcome are visible | `supervisor::record` → `health.status` `incidents`; status panel in `app.js` | Live `health.status` returns *"Healthy, 1 optional note"* with `incidents: []` on a clean machine; incident kinds `service.restarting`, `service.recovered`, `service.restart_failed` include the attempt and the agent's own message | Verified |
| Dependency-aware stack control | `stack.up` / `stack.down` in `devd-services` | Start order is PHP listeners → MariaDB → active web server; stop order is the reverse; tray now calls these instead of looping blindly | Verified by code and tests; no live multi-service failure was induced |
| The Assistant can see health across projects and coordinate ordinary fixes | `health.status` is agent-safe and available to the Assistant; prompt tells it to fix before escalating | Health values returned live to the Assistant surface | Verified |

## 3. System tray (native, state-aware)

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| Real native tray menu, not a mockup | `crates/devd-desktop/src/lib.rs` (`install_tray`, `tray_menu`) | Tauri `TrayIconBuilder` in the installed desktop build; `cargo check` and the desktop contract tests pass; the desktop process runs | Verified by build and contract; menu clicks cannot be driven by browser automation |
| Open/restore, Assistant, runs, browser, www, Quick Share, Services, Start/Stop All, Quick app, Tools, Terminal, Preferences, folders, log, start-at-login, Quit | `tray_menu` | Every row is wired to a capability or a real route; routes corrected to `/diagnostics`, `/server/runtimes`, `/server/services` | Verified by code and contract tests |
| The menu reflects actual state, not a startup snapshot | `live_from_api` + 30-second refresh on the main thread | Each service row shows `running`, `stopped` or `running, not answering`; Start/Stop/Restart are enabled only when they would act; the title shows the running count or incident count | Corrected during this audit; verified by the new contract test and desktop compile |
| Active shares are manageable from the tray | Quick Share submenu built from `share.quick` `links` | Each open link shows its real URL, can be opened or stopped; the submenu label counts active shares | Corrected during this audit |
| Health and incidents are one click away | Always-present Health submenu | Lists recent incidents with detail, says "No incidents" when healthy, links to Diagnostics | Corrected during this audit |
| Start All / Stop All only touch devd-managed services | `stack.up` / `stack.down` | Dependency-ordered; no process is killed by image name from the menu | Corrected during this audit |
| Per-service controls do not stall on an approval the tray cannot show | `service.start` / `service.stop` / `service.restart` | Agent-safe audited capabilities, executed on a worker thread | Corrected during this audit |
| Single instance, start at login, restore from error page | `tauri-plugin-single-instance`, Run key with `--start-hidden`, `show_main` | Code and contract tests | Verified by code; the window was relaunched after each install |

## 4. Portability and relocation

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| Detect that the devd home moved and remember where it was | `crates/devd-agent/src/portability.rs` (`portability.status`) | After copying a home and renaming the old folder: `moved: true`, `previous: C:\dthome-026`, `home: C:\dthome-moved` | Verified |
| Report missing project and site paths | Same | `missing_projects` contained exactly `portable-demo` at `C:\dthome-026\www\portable-demo` | Verified |
| Repair only the paths that moved with the home | `portability.repair` | Returned *"Moved 1 project(s) and 0 site(s) to the new home"*; a follow-up `project.list` showed `C:\dthome-moved\www\portable-demo`; `portability.status` then reported `moved: false`, no missing paths | Verified end to end |
| Separate machine state and secrets from portable configuration | Home layout: `settings.toml`, `state.db`, `share/`, `secrets/`; status reports secrets live under the user profile | `portability.status` returns `settings`, `state_db`, `secrets` and `writable` | Verified |
| Relocation does not need reinstallation | The installed binary was started with `DEVD_HOME` pointing at the moved copy and served the repaired registry | Same run as above | Verified |

## 5. Quick Share (ngrok)

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| Managed ngrok agent, authtoken kept out of argv, regions, verified port | `crates/devd-services/src/ngrok.rs`, caps in `crates/devd-sites/src/caps.rs` | Panel reports *"Setup needed"* with the real binary path, region list and preferences; `share.qr` returns a 7.8 KB SVG drawn locally | Verified up to the tunnel |
| Real URL only, never fabricated | URL parsed from the agent's own log | Code returns an error if no address appears within the timeout; nothing constructs a URL | Verified by code and tests |
| Copy, QR, several shares, stop and cleanup, stale-tunnel pruning | Panel and project/site row Share buttons; `share.ngrok_stop`; `Link` state files | Panel and actions verified in the browser; stop is idempotent and agent-safe | Verified at component level |
| Access protection, region selection | `basic_auth`, `share.settings_set` | Settings round-trip covered by tests; no live tunnel to test protection against | Awaiting verification |
| A real public network round trip | — | No ngrok account or authtoken exists on this machine; the code refuses to invent an address. The managed-agent download was attempted through the real approval path and Windows Defender rejected the download (`os error 225`), so the agent could not even be installed here | Blocked by credentials and the host's antivirus policy |

## 6. Banners, notices and Markdown

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| One shared banner/notice system with severity, icon, spacing and actions | `.notice` / `.banner` rules in `ui/style.css`, `notice-ok/warn/bad/info` tones | Rendered all four states in the running interface: icon masks at 40px, tinted surfaces, 12px radius, wrapping long messages, action button kept inside the panel; live healthy state rendered as a quiet green-check sentence | Verified |
| Accurate wording and no misleading success | Capability outcomes and UI copy | Live diagnostics read *"Healthy, 4 optional notes. Checked just now in 449 ms."*; setup states say *"ngrok needs setting up"* rather than pretending to share; approval failures carry the operating system's real error | Verified |
| Safe Markdown for longer messages | `md()` renderer escapes content before adding structure; no raw HTML injection | Used for Needs-you, run errors, reports, briefs and decision details | Verified |
| Consistent terminology across screens | Shared labels and `capLabel` | Banner states compared across Diagnostics, Sites, Services and run detail | Verified |

## 7. Collapsible spacing and destructive actions

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| More left indentation inside every expanded disclosure | `details > :not(summary) { margin-left: 20px; }` and `.ui-fold` variant in `ui/style.css` | Computed `margin-left: 20px` in the Diagnostics Portability disclosure; applies globally, not per screen | Verified |
| Nested disclosures step again without wasting width | Same rule applies at each level | Nested `<details>` inspected in the same panel | Verified |
| Remove and Remove site use the danger treatment | `btn-danger` on both project row actions in `ui/app.js`; the danger colours are restated after the later base `.btn` block | Computed style on the running 0.25.6 build: `background rgba(242,123,123,.12)`, `border #6b3036`, `color #f27b7b`; screenshot shows Remove red beside neutral Share/Pin | Corrected and verified |
| Other destructive actions are consistently danger-styled | Shared `.btn-danger` rules across project, site, MCP, cron, worker, remote and attachment removal | Stylesheet inspection and contract tests | Verified |

## 8. Composer, effort and speech

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| Model effort selection Low/Medium/High/Extra/Max, wired to inference | `EFFORTS`, `composer-effort`, `effort_set` in `ui/app.js` and provider request bodies | Opened the live model picker on the running desktop backend: Effort page listed Low, Medium (Recommended, selected), High, Extra, Max; the composer bar showed `DeepSeek-V4.1-FL… Medium`; `effort_settings` maps the choice to `reasoning_effort` and the answer's token room | Verified |
| Handy-style local speech-to-text | Browser recording at 16 kHz mono WAV, `dictation.transcribe` → Whisper (whisper.cpp) locally | The composer shows the microphone control; `dictation.status` honestly reports *"speech to text is not installed yet"* with the model catalogue and the whisper path. The ~152 MB model was not installed in the live home | Awaiting verification (model download) |
| Microphone → transcription → composer flow | `composer-speech`, `dictation.transcribe`, error toast | Wired end to end in code and UI contract tests; not exercised with real audio here | Awaiting verification |

## 9. Token efficiency and performance

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| Relevant context rather than whole histories | `build_messages`, chapter summaries and tool-result clipping in `devd-agent` | Agent tests around message construction and context; DeepSeek tool-pairing fix has regression tests | Verified at component level |
| Reuse findings and structured handoffs | Run steps, evidence and specialist reports are stored and read back | Run detail renders evidence and reports; `team.overview` exposes dependencies and evidence | Verified |
| Actual provider usage, not estimates | `tokens {in,out,cached}` from provider usage, surfaced per agent | Live `team.overview` returned measured counts | Verified |
| No unnecessary polling | Tray refreshes only when the snapshot changes; supervisor ticks on an interval; UI subscriptions are event-driven | Code inspection; tray diffing added in this pass | Verified by code |
| Benchmarks | No standalone benchmark harness exists in the repository | Not fabricated | Incomplete (no benchmark suite; performance checked qualitatively) |

## 10. Build, tests and release

| Check | Command | Result | Status |
| --- | --- | --- | --- |
| Desktop compiles with the live tray | `cargo check -p devd-desktop` | Finished | Verified |
| Workspace tests | `cargo test --workspace` | All suites green: agent 170, server 255, ui_contract 102, services 125, plus the stack, workspace, sites, ops, TUI, CLI, PHP, doctor, runs and HTTP suites. No failures. | Verified |
| Contract tests for this pass | `cargo test -p devd-server --test ui_contract` | 102 passed, including live tray state, dependency-ordered Start/Stop, active-share stop, health, and the danger-rule ordering | Verified |
| Production build | `scripts/release-windows.ps1 -SkipTests` | Produces `devd` + desktop binaries and a zip; smoke test and `devd doctor` pass | Verified |
| Installed build matches the published assets | SHA-256 comparison after install | devd and desktop hashes match the release files; registry and `--version` report 0.25.6 | Verified |
| Browser inspection | Test server + installed desktop backend | Projects (Share/Remove), Diagnostics (folds 20px, portability), banners (four states), composer effort (five choices), empty/healthy states, console | Verified |
| Tray actually installs | Installed desktop log | `tray icon ready` on the final 0.25.6 build; process stays alive and serves its backend | Verified by runtime log |
| Native tray clicks | Manual desktop interaction | Cannot be scripted by the browser tool; menu construction and wiring are covered by compilation and contract tests | Blocked by harness |

## 11. Genuinely blocked

* **A live ngrok tunnel and public URL.** No ngrok account or authtoken is
  available here, and Windows Defender blocks the ngrok download on this host
  (`os error 225: contains a virus or potentially unwanted software`). The
  agent, token storage, region selection, port verification, URL parsing, QR
  code and cleanup are implemented and tested; the public round trip cannot be
  demonstrated in this environment.
* **Tray menu clicks.** The in-app browser cannot drive operating-system tray
  menus. The menu's construction, wiring, live refresh and routes are covered by
  compilation and contract tests.
* **Speech model download.** The local Whisper model is ~152 MB and was not
  fetched during this pass, so microphone-to-text was not exercised on real
  audio.
