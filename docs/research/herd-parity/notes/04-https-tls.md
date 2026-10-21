# 04 — Local HTTPS (P12, S02)

**Evidence.** Capabilities: `tls.ca.ensure`, `tls.ca.show`, `tls.ca.status`, `tls.issue`, `tls.list`, `tls.trust` and `tls.untrust` (the last two Admin, approval required). Windows trust uses `certutil -addstore`; Linux uses `trust anchor` or prints a manual recipe (`docs/limitations.md`). `tls.rs` has 7 `cfg` gates; leaves are ECDSA P-256, valid 397 days. Herd keeps its CA under `config/certificates/` and offers `secure`, `unsecure`, `secured`.

**Verified.** The issue/list/trust plan-and-approve flow exists. Chain verification against devd's own CA is claimed with tests in the prior matrix; not re-run individually this session (the suite passed overall).

**Gaps.** No per-site unsecure capability. macOS `security add-trusted-cert` is absent. Expiry display unverified (U-06). Firefox/NSS trust is not mentioned in the code read.

**Tests to add.** macOS trust command plan test with a fake runner; per-site unsecure removes the leaf but not the CA.
