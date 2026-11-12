---
name: threat-model
description: Find how a system can be abused and the smallest mitigations
kind: design
roles: security,architect
---

# Threat model

A one-page analysis of how a system can be abused: what is worth attacking, where input arrives, where trust changes, and what closes each gap. It records only what you verified.

## When to use

A new feature that takes input, stores data, runs commands or calls out to the network, and before a release of one.

## Procedure

1. **Assets.** What would hurt to lose, leak or corrupt: credentials, personal data, money, the ability to run code, the audit trail.
2. **Entry points.** Everywhere input arrives: routes, commands, files, queues, webhooks, environment, other programs on the machine.
3. **Trust boundaries.** Where data crosses from less to more trusted, and who is on each side. A boundary with no check is a finding.
4. **Abuse cases.** For each entry point, one sentence: "someone who can ... can make it ...". Cover injection, missing authorisation, replay, path tricks, server-side requests to addresses the caller picks, and resource exhaustion.
5. **Mitigations.** The smallest change that closes each case, and whether it is already there. Prefer making the invalid state impossible over detecting it.
6. **Residual risk.** What remains, why it is acceptable, and who accepted it.

## Pitfalls

- Listing generic threats that do not apply to this system.
- Claiming a mitigation exists without having read it.

## Verification

Each claim cites a file, line or command output, and the page says what you did not look at.
