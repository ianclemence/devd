# 02 — Sites, domains and routing (P03–P05, P08–P12, S03–S05)

**Workflow.** Herd: `herd list` inventory (link, park, proxy, tld, loopback, directory-listing) plus prior notes on HerdHelper and dnsmasq. DEVD: grep for each feature; read `site.create` and `site.set_php`.

**Evidence.** DEVD's site capabilities are exactly `site.create`, `site.list`, `site.remove`, `site.set_php` (`devd capabilities`). There is no `proxy_pass`, `autoindex`, loopback setting, park, wildcard or `http2` anywhere in `crates/` (greps). `hosts.sync` is `Admin` tier, snapshots the file first and preserves content outside its block (`crates/devd-sites/src/hosts.rs`). A site is a PHP docroot served through nginx FastCGI.

**Verified.** Create, list, remove and pin-PHP exist with tests (`crates/devd-sites/src/lib.rs`). The absence of proxy, park, directory-listing and loopback is a grep finding across crates, UI and YAML, consistent with `docs/limitations.md` and `../02-feature-matrix.md`.

**Intentional divergence.** Hosts writing is an explicit, approved, elevated operation; Herd uses a resident Windows service. DEVD's choice is safer but costs an elevation per publish.

**Uncertain.** Whether `site.list` reports HTTPS and Node version (U-06). Changing the TLD is a setting, not a capability that re-issues certificates and rewrites hosts atomically.

**Tests to add.** Proxy vhost rendering (upstream validated, loopback-only by default); wildcard resolver plan; `directory-listing` renders `autoindex off` by default.
