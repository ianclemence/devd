# Cross-stack analysis

Stack definitions: `crates/devd-stacks/defs/stacks/*.yaml` (23 files) and the
generated [`docs/stacks.md`](../../stacks.md). "Template available" is not the same as
"the surrounding environment exists" — each row below separates the two.

## 1. Applicability of each Herd feature

| Class | Features (matrix IDs) | Treatment |
|---|---|---|
| **Universal** | P01–P04, P06, P07, P11, P13, S01–S07, C01–C03, D01, D03, D08, I01, I02, I04, M01–M04 | Build once in core; stack adapters supply only launch/log paths |
| **Language-specific** | R01–R07 (PHP, Node), D05 (Xdebug) | Per-runtime provider behind the `runtime.*` contract |
| **Framework-specific** | D06 (tinker), D07 (dev stacks) | Stack definition supplies the command (`rails console`, `python -m …`, `npm run dev`) |
| **Service-specific** | V01–V08 | One service contract; one definition per service |
| **Laravel-specific** | D02 (dumps), D04 (insights), laravel installer | **Do not impose.** Offer only as an optional adapter if Laravel users ask |
| **Platform-specific** | S02–S04, M03–M05, Z01–Z02 | Platform adapters under one trait |

## 2. Per-stack status (what exists today vs the infrastructure a project of that stack expects)

| Stack (def) | Create / detect / run | Runtime provided by devd | Service it typically needs | Domain+HTTPS (`.test`) | Gaps beyond Laravel |
|---|---|---|---|---|---|
| Laravel, Slim (PHP) | yes | PHP: **Windows only** | MariaDB ok (Win); MySQL/Postgres/Redis **not managed**; mail **absent** | yes via nginx+FastCGI (Win) | Linux/macOS PHP; `.env`→service wiring; mail capture |
| Next, Nuxt, Astro, Vite/React, Vue, SvelteKit, Express, Node | yes (npm/pnpm/yarn/bun) | Node: Win+Linux | Postgres/Redis common, **not managed** | **no**: site model is nginx+PHP docroot; no reverse proxy (P11) | Dev servers get no `name.test`/HTTPS; no port→domain mapping |
| Django, Flask, FastAPI, Python | yes (pip/uv) | **Python not provided** (system/winget/apt via `toolchain.*`) | Postgres/Redis **not managed** | **no** (needs proxy) | No managed Python; no venv-aware runtime selection |
| Rails, Ruby | yes | **Ruby not provided** | Postgres/Redis | **no** | same |
| Go | yes | Go: Win+Linux | any | **no** | same |
| Rust | yes | system via rustup/`toolchain` | any | **no** | same |
| Spring Boot, Java | create (Initializr) / detect+run | **JDK not provided** | Postgres/MySQL | **no** | Heavy-service stacks have no DB at all |
| ASP.NET Core | create, run | **dotnet not provided** | SQL Server (not in tools.yaml), Postgres | **no** | SQL Server absent |
| Expo, React Native | create, run, ADB/EAS surfaced | Node | — | n/a | device/emulator flows are `toolchain.*` only |

Evidence: stacks table and tools table in `docs/stacks.md`; PHP-only site model in
`devd-sites/src/caps.rs` (`site.create` is a PHP docroot site) and `devd-services/src/nginx.rs`;
absence of `proxy_pass` verified by grep (P11).

## 3. Reusable abstractions that would serve several stacks

| Abstraction | Serves | Where it lives today | Change |
|---|---|---|---|
| **Site = domain + upstream** (FastCGI *or* HTTP proxy *or* static) | every web stack | `site.*` assumes docroot+PHP | Generalise the site record to carry an upstream kind (P11, C01) |
| **Service instance contract** (type, version, port, data dir, health) | DBs, caches, mail, search | `devd-services` (nginx, MariaDB) | Add definitions, not new code paths (V02) |
| **Project requirements** (runtime + services + env keys) | all | `project.detect` reports runtimes/services | Persist as part of `devd.yml`, add `.env` wiring (V06, C01) |
| **Log source registry** (path + parser per stack) | all | scattered tails | One discoverer, stack-supplied path and parser (D03) |
| **Capture sink** (SMTP, HTTP webhook) | all | none | Mail first (D01); webhook receiver later |
| **Platform provider trait** (catalogue, install, trust, privileged hosts, autostart) | all | `cfg!(windows)` branches in catalogues | Introduce per-OS providers (Z01/Z02) |

## 4. Where Herd's design should *not* be copied

- A second manifest (`herd.yml`) beside `devd.yml`. Extend `devd.yml` instead.
- Pro-gated services — DEVD's equivalent is ordinary engineering, as
  `../01-herd-investigation.md` §4.4 notes.
- Hosts changes through a resident privileged service as the only path; keep
  `hosts.sync` approval-gated and add a DNS resolver on Unix instead.

## 5. Caveats

Stack rows reflect definitions and tool recipes, not a per-stack runtime test
(see [unknowns](unknowns-and-follow-ups.md) U-07). "Not provided" means no devd-managed
download exists; `toolchain.*` may still install it via the OS package manager.
