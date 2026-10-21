# 05 — Managed services (S01, S06, S07, V01–V04, V09)

**Evidence.** `service.*` (9 capabilities) manage nginx and MariaDB. `services.inventory` and `services.decide` implement adopt/observe/external/ignore (`crates/devd-stacks/tests/engine.rs:321,360`). Health means the port answers and the pid image matches (`crates/devd-services/src/lib.rs:197-219`, per the prior doc). MariaDB provisioning in `mariadb.rs` (483 lines) has 6 unit tests (`#[test]` count, this session); the prior matrix saying "none" is stale. Coverage of init, root password and `test`-schema removal was not checked. Providers are Windows-only (`crates/devd-services/src/catalogue.rs:325,333`). `service.remove` keeps the data directory, unlike Herd's `services:delete`.

**Herd comparison.** Herd offers instance create, clone and delete for mysql, mariadb, postgresql, mongodb, redis, valkey and typesense, tagged Pro in its CLI. Mechanically this matches DEVD's provider shape.

**Verified.** Start, stop, health and remove-preserves-data for nginx and MariaDB on Windows. Nothing else is verified.

**Tests to add.** Fill any gaps in MariaDB init, root-password and `test`-schema coverage; a per-service definition conformance test as services are added.
