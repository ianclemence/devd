# Herd reference catalogue (2.0.2, Windows)

Labels: **[CLI]** read from `herd list` this session; **[CFG]** read from Herd's
config files this session; **[REA]** REA this session; **[PRIOR]** from
[`../01-herd-investigation.md`](../01-herd-investigation.md) (REA 6.7.0 ASAR/preload
analysis, DevTools-driven screen capture); **[INF]** inference.

## Environment

| Item | Value | Source |
|---|---|---|
| Version | 2.0.2 (`Herd.exe` ProductVersion 2.0.2.0) | [CLI] [REA] |
| Platform | Windows 10 Pro 19045, x64 | host |
| Shell | Electron; main process compiled to V8 bytecode (`.jsc`) — not decompilable | [PRIOR] |
| Fuses | RunAsNode on, NodeOptions env on, Inspect args on, ASAR integrity validation **off**, OnlyLoadAppFromAsar off | [REA] |
| Edition | **Unknown.** `validLicense` not read. `services:*`, `debug:*` and `insights:list/analyze/capture` self-describe as Pro in `herd list`; `mail:*` carry no Pro tag in that text (earlier research called mail Pro — unverified) | [CLI] |
| Config home | `~/.config/herd/` (`config.json`, `php.json`, `herd.json`, `valet/`, `nginx/`, `certificates/`, `herd.sqlite`) | [CFG] [PRIOR] |
| Ports | API 9001, base PHP 9000 (+100 debug), SMTP 2525 | [CFG] |
| Installed PHP | 8.5.11 only (`installed_8.5`) | [CFG] |

## Command surface (complete, from `herd list`) [CLI]

| Group | Commands |
|---|---|
| Sites | `link unlink links park forget parked paths sites open edit which site-information directory-listing` |
| HTTPS / domains | `secure unsecure secured tld loopback proxy unproxy proxies` |
| PHP / Node | `use isolate unisolate isolated which-php php composer ini php:install php:list php:update node:install node:list laravel:update` |
| Services (Pro) | `services:available versions create list start stop clone delete` |
| Database | `db db:connection db:create db:drop db:list db:use` |
| Mail | `mail:list latest show links mailboxes wait config clear delete attachment attachments` |
| Debug / insights (Pro) | `debug:start debug:stop insights:list analyze capture clear status watch` |
| Logs / processes | `log logs:files logs:read processes processes:start processes:stop processes:output` |
| Project config | `init init:fresh` (herd.yml) |
| Workspaces | `workspace workspaces workspace:open switch merge delete` |
| Agents | `mcp mcp:install mcp:uninstall mcp:status studio tinker` |
| Sharing | `share fetch-share-url` (Expose) |
| Lifecycle | `start stop restart status` |

## Behaviours worth specifying

| ID | Behaviour | Evidence |
|---|---|---|
| H-A1 | "Link Existing App" opens the OS folder dialog, returns a path; no typing required | [PRIOR] §6 |
| H-A2 | `link` serves a dir as `name.test`; `park` serves every subdir | [CLI] |
| H-B1 | PHP versions installed/updated side by side; global default + per-site isolation; patch-level update flow tracked in `php.json` | [CLI] [CFG] |
| H-B2 | Node via bundled nvm-windows; per-site Node version shown in `parked` | [CLI] [PRIOR] |
| H-B3 | `php`/`composer` shims run with the PHP version that serves the *current site* | [CLI] |
| H-C1 | nginx on 127.0.0.1:80/443, PHP via FastCGI port 9000+offset, per-site snippets | [PRIOR] |
| H-C2 | Hosts-file edits go through the `HerdHelper` Windows Service so no UAC prompt per change | [PRIOR] |
| H-C3 | Local CA, per-site certs, `secured` lists expiry; `tld`/`loopback` configurable | [CLI] |
| H-D1 | Service instances: type+version+port+data dir; clone; delete removes data | [CLI] (Pro) |
| H-D2 | `db:*` operate on whichever server the site's `.env` names | [CLI] |
| H-E1 | `herd.yml`: `name, php, secured, aliases, services{type: version, port: ${ENV}}, integrations`; validated with one message per problem | [PRIOR] §4.2 |
| H-E2 | Workspaces = git worktree served on its own `.test` domain with own certificate; merge back | [CLI] |
| H-F1 | Local SMTP (2525) capture, mailboxes keyed by SMTP username, `mail:wait` blocks for new mail | [CLI] [CFG] |
| H-F2 | Dump interception (`dump()/dd()`), query/log/job/HTTP/cache capture, N+1 analysis | [CLI] [CFG] |
| H-F3 | App log viewer parses `storage/logs/*.log` into level/message/stack | [CLI] |
| H-F4 | Xdebug session capture (`debug:*`), `ini --debug` | [CLI] |
| H-G1 | `mcp:install` writes Herd's MCP server into 8 agents' configs, global or per project | [CLI] |
| H-G2 | Herd Studio: browser devtools + embedded agent over a WebSocket helper | [PRIOR] |
| H-H1 | Tray-first app, autostart, updater, Defender exclusion hint, HerdHelper service | [PRIOR] [CFG: `autoStart`, `defenderExclusionHintShown`] |

## Screens [PRIOR]

Sixteen renderer windows; the ones that matter: Applications (site list/grid),
Settings (General, PHP, JavaScript, AI, Services, Herd Pro, Sharing, Shortcuts,
About), Services, Mails, Dumps, Logs/Log Viewer, Onboarding, Updater, What's New.
Full measurements are in `../01-herd-investigation.md` §3 and are not repeated.

## Limits of this inventory

Main-process logic is V8 bytecode, so behaviour is inferred from the preload
contract, CLI, config and logs — never from main-process source. Pro features were
not exercised. Screens were captured by the earlier investigation and **not**
re-captured this session.
