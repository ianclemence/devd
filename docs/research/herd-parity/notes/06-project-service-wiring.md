# 06 — Project ↔ service wiring and databases (V05–V08)

**Evidence.** Herd: `db:connection`, `db:create`, `db:drop`, `db:list`, `db:use` operate on whichever server the site's `.env` points at. Its `herd.yml` services accept `${ENV}` ports and report missing variables by name (`Manifest.php`, prior §4.2). DEVD: `db.list/create/drop` act on devd's MariaDB only (`crates/devd-services/src/caps.rs`); nothing reads a project's `.env`. `../04-implementation-plan.md` §1b describes the intent (`site.services`, `site.set_service`), but no such capability exists in the 234-entry registry. That plan item is documentation-only.

**Why it matters.** It applies to every stack with an `.env` or equivalent (Laravel, Rails, Django, Next, Express and others). It is the highest-value universal gap.

**Open questions.** Secret handling: report keys as set or unset, never values. Port-conflict handling (U-09).

**Tests to add.** Round trip: set then unset restores the original values; secrets are absent from audit rows; an unknown service is rejected with one message per problem.
