# Implementation plan for the remaining work

The order below is the order the work will be done in, and the reason for it.
Each item states what "done" means, so the work can be checked rather than
asserted.

---

## 1. Managed services, and per-site service configuration

**Why first.** It is the largest gap between devd and the Herd feature set, and
two parts of it are independent of any new vendor download.

### 1a. Per-site PHP version routing (Herd's `isolate`)

The data model already supports it: `sites.php` holds the version a site was
created against, and `site.create` writes the primary runtime's version. The
nginx renderer ignores it and sends every site to one FastCGI upstream.

**Done means**

* One `php-cgi` listener per installed PHP version, each on its own port
  (`9000 + major * 10 + minor`, so 8.3 is 9083 and 8.5 is 9085 — the same scheme
  Herd uses).
* The generated nginx configuration has one upstream per version, and each
  site's server block points at the upstream for the version it is pinned to.
* `site.set_php` — pin a site to an installed version, or unpin it back to the
  primary. Refuses to pin to a version that is not installed, and says which
  versions are.
* `site.list` reports the pinned version, and the Sites screen and the CLI
  (`devd site php <name> <version>`) expose it.
* `stack.up` starts every listener; `stack.down` stops them.
* Tests: the rendered configuration points a pinned site at its own upstream and
  an unpinned one at the default; a site pinned to a version that is not
  installed is refused.

**Why it is first.** It closes a documented limitation, it has a clear
definition of done, and it needs no download.

### 1b. Per-site service configuration (Herd's `site:services`)

**Done means**

* `site.services` — read a project's `.env` and report, per category
  (database, queue, cache, broadcasting), which service it points at, the
  driver, host, port and database, and whether the value is one devd manages.
* `site.set_service` — repoint a category at a chosen instance, rewriting only
  the keys that category owns.
* `site.unset_service` — put the original values back. The values that were
  replaced are remembered, so "unmanaged" really does restore what was there.
* Secrets are never returned in a listing; only the fact that a key is set.
* Tests, including one that proves the original values come back.

### 1c. PostgreSQL and MongoDB as devd-managed services

**Done means**

* A catalogue for each, from the vendor's own listing, with a digest where the
  vendor publishes one and an explicit warning where it does not — the same
  honesty the nginx installer already has.
* A definition, a data directory, an initialisation step, an argument array
  and a start/stop path, each with a real health check (a protocol-level
  answer, not a bare TCP connect).
* Coexistence: an installation found on the machine is never replaced, and the
  same `services.decide` records apply.
* On Linux these come from the system package manager, which the toolchain
  already does; the devd-managed path is Windows.

**Known constraint, stated now.** Redis has no official Windows build, and
Valkey publishes no Windows binary either. A devd-managed Redis on Windows would
mean an unofficial build, which this product does not do. On Linux both are
available through the package manager.

### 1d. One service-control story

`service.*` and `toolchain.*` are two stacks that manage overlapping things at
different tiers: starting nginx is `Exec` and starting PostgreSQL is `Admin`.
**Done means** the tiers agree, the two are documented as one story, and the
Services screen shows the decision (managed by devd, watched, or yours) beside
the state.

---

## 2. Local mail capture

**Done means**

* A local SMTP server on `127.0.0.1:2525` that accepts a message, stores it,
  and answers. It is a service devd supervises, like any other.
* A `mails` table (schema migration 6) holding the message, its headers, its
  recipients and its attachments.
* A Mail screen: the mailboxes, the messages, the message itself, the
  attachments, and a way to delete one. A mailbox is the SMTP username; mail
  sent without one lands in "Default".
* `mail.config` — the host, port and the `.env` values an application needs to
  send into it.
* `mail.wait` — block until a message matching a filter arrives.
* A real limit: the mailbox is pruned on a policy, not forever.

**Constraint.** A local mail server receives anything sent to it. It binds to
loopback only, and the screen says plainly that it is a development catcher,
not a mail server.

---

## 3. The design-system debt

**Done means**

* Every `px` literal in the stylesheet after the token block is either a token
  or is explained. The remaining literals are folded in.
* The inline `style` attributes in the interface are gone; the values they
  carried are tokens.
* The dead tokens and the dead rules are removed.
* The `100vh - Npx` heights are replaced by a layout that does not know the
  height of the chrome above it.
* The four tab strips are real tabs: `role="tablist"`, `role="tab"`,
  `aria-selected`, and arrow-key navigation.
* Every control that is identified only by its placeholder has an accessible
  name.
* Focus returns to whatever opened the palette, the dialog or the drawer.
* The palette's input has a focus indicator.
* The terminal uses the same colour system as the window. The xterm theme is
  generated from the tokens, not written out again.

---

## 4. Terminal parity

**Done means**

* The terminal dashboard has a **Server** view with Sites, Services, Runtimes,
  Certificates and Hosts, reached the way the window reaches them.
* A folder browser in the terminal: `devd project browse` already lists a
  directory; the dashboard should offer the same thing when it needs a path.
* Every screen reads the same capabilities the window does, so the two cannot
  disagree.

---

## 5. Reliability

**Done means**

* The SQLite and PTY calls in the HTTP layer run off the Tokio workers, like
  the registry dispatch already does.
* The audit table, the chat events, the team events and the log files are
  bounded by a policy rather than growing without limit, and the policy is
  documented.
* The plan computation no longer writes an audit row every time somebody asks
  what an operation would do — a plan is a question, not an attempt.

---

## 6. Live preview

**Done means**

* The Preview screen shows the site's **requests** as they happen, from the
  access and error logs nginx is already writing, with the status, the time and
  the URL.
* Application errors are surfaced, parsed out of the site's PHP error log,
  with the file and line when the log has them.
* The **file changes** a preview session produced are listed, with a readable
  diff, and a person can review them before accepting them.
* Element picking and source linking are **not** done by injecting script into
  the site. If they are done, the site is not modified. The honest version is
  the server-side one above.

---

## 7. Tunnels and self-update

**Done means**

* **Self-update**: devd checks the release list, shows what is available,
  downloads the artifact, verifies it against the published checksum, replaces
  the binary, and rolls back if the new one does not start.
* **Tunnels**: only with real access control — a token, an explicit opt-in, a
  visible indicator, and a way to revoke. Until that exists, devd offers a
  **local webhook receiver** instead: a loopback port that captures requests
  and shows them, which is the half of the feature that is safe to ship.

---

## What is deliberately not in this list

* **A native picker in the terminal.** A terminal has no native dialog; a
  listing and a path are the honest equivalent, and they exist.
* **Per-site PHP versions served by a different web server.** nginx is the
  server devd drives; Apache would be a second implementation of everything
  above.
* **A mail server that relays.** devd catches mail for development. Relaying
  mail is a different and much larger product.
