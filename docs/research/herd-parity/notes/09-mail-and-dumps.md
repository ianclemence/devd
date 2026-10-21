# 09 — Mail capture and dumps (D01, D02)

**Evidence.** Herd: SMTP server on port 2525 (`config.json` `smtpPort`), SQLite tables Mails, MailHeaders, MailAddresses, MailAttachments, `mail:*` CLI including `mail:wait`, and a dump server for `dump()`/`dd()` (`dumps: true`). DEVD: no SMTP code, no mail tables in `sql/001`–`005`, no dump interception (greps for `smtp` and `SMTP` are empty).

**Classification.** Mail: not implemented, universal value (any stack sends mail). Dumps: PHP/Laravel-specific; defer.

**Design guardrails.** Bind loopback only; bounded retention; mailbox is the SMTP username; render messages as text by default and sanitise links and HTML in the UI.

**Tests to add.** An SMTP client send appears in the list; `mail.wait` timeout exit code; retention prune; attachment path traversal rejected.
