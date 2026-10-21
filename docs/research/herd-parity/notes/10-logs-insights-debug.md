# 10 — Logs, insights, Xdebug, tinker (D03–D06)

**Evidence.** Herd: `logs:files` and `logs:read` (parsed Laravel logs), `insights:*` (Pro; N+1 analysis), `debug:*` (Pro; Xdebug), `tinker`. DEVD: `service.logs`, `project.logs`, `stacks.log` — raw tails of things devd runs. There is no discovery of an application's own log files, and no Xdebug or tinker.

**Classification.** Logs: partial, universal opportunity (each stack supplies paths and parsers). Insights and Xdebug: Laravel/PHP-specific, not started. Tinker: a framework REPL; DEVD agent sessions give a shell, and a cross-stack REPL launcher is possible.

**Uncertain.** Pro gating prevented observing insights and debug behaviour (U-03).

**Tests to add.** A log discoverer returns each stack's conventional paths; the parser handles multi-line stack traces; follow mode is bounded.
