# The resident process

One devd process per user owns the work that must keep happening whether or not
a window is open, and serves the local API the window, the terminal dashboard and
the command line talk to. It is `devd daemon`.

```text
devd daemon run     in this terminal, until Ctrl+C
devd daemon start   detached, no window; waits until it answers
devd daemon stop    stops it; services it started keep running
devd daemon status  running, where, since when (exit 1 when it is not)
devd autostart enable   start it at sign-in
```

`autostart enable` uses what each platform gives a user without administrator
rights: a `Run` value in the registry on Windows, a `systemd --user` unit on Linux
(an XDG autostart entry where there is no user manager), a LaunchAgent on macOS.

## Finding it: the instance record

`<home>/instance.json` holds the process id, the process's **start time**, its
image name, the port of its local API, the version and a role (`daemon` for the
resident process, `ui` for a window's own server). It is created with an
exclusive create, so of two processes starting together exactly one wins.

A record is believed only when the process it names is alive, is the same
program, **and started at the recorded time**. A recycled process id, or the
leftover file of a crashed daemon, is never mistaken for a live one, and is
replaced by the next claim. No port is scanned to find the instance.

`devd ui`, `devd serve` and the desktop window read the record. If a daemon is
running they attach to it (the window loads its interface and owns no background
work); if none is, the process becomes the instance for as long as it runs.
A record whose process is alive but whose port does not answer is reported as a
conflict, never silently replaced: starting a second server beside a hung one
would split the background work between two processes.

## What it does

Every 15 seconds: cron jobs, scheduled backups, scheduled deploys, health checks.
Every hour, and once at start:

* **Log rotation.** A `.log` file under `logs/` or `etc/` over 10 MB is copied to
  `<name>.log.1` (older copies shift up; five are kept) and emptied in place. It
  is copied and truncated rather than renamed because a running service keeps its
  log open and Windows will not rename an open file. Lines written between the
  copy and the truncation are lost. Settings: `logs.max_bytes`, `logs.keep`.
* **Audit retention.** Audit rows older than 365 days are removed
  (`audit.retention_days`; `0` keeps everything).
* **Download cache.** Cached downloads unused for 90 days, and then the least
  recently used until the cache fits in 2 GiB, are removed. Stale partial
  downloads go after a week. Only downloaded archives and installers are touched
  (`cache.max_age_days`, `cache.max_bytes`).
* **Undo snapshots.** Snapshots older than 30 days, then the oldest beyond 1 GiB,
  are removed (`snapshots.max_age_days`, `snapshots.max_bytes`).
* **Service reconciliation.** See below.
* **Local DNS** (started once, below).

## Owning services: process image and start time, not ports

When devd starts a service it writes its pid and the process's start time. Later,
from any devd process, a service counts as devd's only if the pid is alive, the
program is the one the service runs, and it started when the record says. The
reconciliation pass compares the records with what is running and reports one of:

| Finding | Meaning |
|---|---|
| `adopted` | devd started it and it is still running; it is managed. |
| `stale_record` | the pid record names a process that is gone or is not devd's; the record was removed. |
| `ambiguous_owner` | something holds the service's port, but devd cannot show it started that process; it is **left alone**. |
| `down` | the service should be running and nothing is. |
| `unexpectedly_running` | it is running although it was meant to be stopped. |

A port is never evidence of ownership, so devd never adopts or stops a stranger
that happens to listen where nginx should.

## Stopping what is in flight

Stopping a run reaches the program a handler is waiting on. The dispatcher
publishes the run's cancellation to the thread executing the handler, and
`util::run` (which every handler uses to start a program) checks it a few times a
second: when it fires the whole process tree is killed and the call returns a
`cancelled` error. A handler that moves its work to another thread must pass the
answer along itself.

## Local names without the hosts file

The daemon starts a small DNS server on `127.0.0.1:5354` that answers `A` and
`AAAA` for the `.test` suffix (`dns.suffixes`) with loopback and refuses
everything else. It never forwards a query. `dns.status` reports two different
facts: whether that server answers, and whether the operating system asks it.

Pointing the system at it changes how the whole machine resolves names and needs
administrator rights, so `dns.enable` (human only) shows the exact commands and
runs only those: `/etc/resolver/test` on macOS, `resolvectl` on Linux
(systemd-resolved), a name-resolution policy rule on Windows, which can only send
a suffix to port 53. If a command is refused, nothing is claimed and the hosts
file remains how names resolve. `dns.disable` undoes it.

## Updating devd

`devd update check` compares the running version with the latest release.
`devd update apply` downloads it only if:

1. the release carries `SHA256SUMS.txt` and `SHA256SUMS.txt.sig`;
2. the signature verifies, with the Ed25519 key compiled into the program
   (`RELEASE_PUBLIC_KEY`), over the checksum file;
3. the program's SHA-256 matches the signed checksum;
4. the downloaded program runs and reports the version being installed.

Only then is the installed program replaced, and the old one is kept beside it as
`devd.before-<version>`. GitHub and the network are trusted for delivery only.
This is not a code-signing certificate: Windows SmartScreen still warns about an
unsigned installer, and the signature is what lets devd check its own updates.

## Undo

Before `fs.write`, `fs.edit` or `fs.delete` replaces or removes something, and
before `db.drop` drops a database, what would be lost is saved (a file up to
32 MB, a directory up to 5,000 files or 128 MB, a database as a dump) and the
result carries an `undo` id. `snapshot.list` shows them; `snapshot.restore` (human
only) puts one back and first saves what it replaces, so an undo can be undone.
Secrets files are never copied. What is not saved is stated in the result:
`undo_note` says why. `db.drop` refuses to proceed if its dump cannot be made,
unless `no_snapshot` says to.
