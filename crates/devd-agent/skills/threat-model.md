---
name: threat-model
description: Find how a system can be abused: assets, entry points, trust boundaries, abuse cases and the smallest mitigations.
kind: technique
roles: security,architect
---

# Threat model

Keep it to one page. Work through it in order.

1. **Assets.** What would hurt to lose, leak or corrupt: credentials, personal data, money, the ability to run code, the audit trail.
2. **Entry points.** Everywhere input arrives: routes, commands, files, queues, webhooks, environment, other programs on the machine.
3. **Trust boundaries.** Where data crosses from less trusted to more trusted, and who is on each side. A boundary with no check is a finding.
4. **Abuse cases.** For each entry point, one sentence: "someone who can ... can make it ...". Cover injection, missing authorisation, replay, path tricks, server-side requests to addresses the caller picks, and resource exhaustion.
5. **Mitigations.** The smallest change that closes each case, and whether it is already there. Prefer making the invalid state impossible over detecting it.
6. **Residual risk.** What remains, why it is acceptable, and who accepted it.

Only record what you verified in code or in a command's output. Say what you did not look at.
