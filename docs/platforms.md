# Platforms: Windows, Linux and macOS

devd runs on all three, but it gets its web server, database and PHP differently
on each, because the people who publish them do.

| | Windows | Linux | macOS |
|---|---|---|---|
| nginx, MariaDB | downloaded from nginx.org / mariadb.org, checked against a digest | **adopted** from the system package | **adopted** from Homebrew |
| PHP | downloaded from windows.php.net, checked against php.net's digest | **adopted** from the system package | **adopted** from Homebrew |
| Node.js, Go | downloaded, checked against the vendor's digest | downloaded, checked against the vendor's digest | downloaded, checked against the vendor's digest (darwin builds; **not run on a Mac**) |
| Trust the local CA | `certutil` | p11-kit `trust` | `security add-trusted-cert` |
| Default ports | 80 / 443 | 8080 / 8443 | 8080 / 8443 |

## What "adopted" means

Linux and macOS have no publisher for nginx, MariaDB or PHP that devd could
download a verified archive from: the distribution (or Homebrew) builds and signs
them. devd therefore does **not** fetch a second copy. When you run
`devd service install nginx` (or `mariadb`, or `devd runtime install php`):

1. It looks for the program the package manager installed. If it is missing, the
   error says so and points at `devd toolchain install <tool>`, which shows a plan
   and asks for approval before it touches the system.
2. It asks the package manager which package owns the file and whether the file
   still matches that package:
   * Debian, Ubuntu: `dpkg -S`, `dpkg --verify`
   * Fedora, RHEL: `rpm -qf`, `rpm -V`
   * Arch: `pacman -Qo`, `pacman -Qkk`
   * Homebrew: the owning formula from the `Cellar` path. Homebrew keeps no record
     to re-check afterwards, so devd says that instead of claiming a check.
3. **A file that differs from its package is refused.** A file no package owns is
   allowed, with a warning that nothing vouches for it.
4. It links the program into devd's own layout (`services/nginx/sbin/nginx`,
   `runtimes/php/<version>/bin/php`), so every other part of devd finds it where
   it expects to. The install manifest records `"source": "system"`, the package
   and its version.

This is the same rule as on Windows: *a program devd cannot account for is a
program it does not run*, with the package manager standing where the
publisher's digest stands.

Configuration and data stay devd's own. devd never reads or writes `/etc/nginx`,
`/etc/mysql` or `/etc/php`; it generates its own `nginx.conf`, `my.ini` and data
directory under its home. Distribution services are not started, stopped or
reconfigured.

### Details that differ from Windows

* **nginx** needs `mime.types`, `fastcgi_params` and writable temporary
  directories under its prefix. devd copies the system's two files when it finds
  them and writes standard ones when it does not, and points nginx's temporary
  paths at its own directory, because a package's default (`/var/lib/nginx`)
  cannot be created by an ordinary user.
* **MariaDB** is initialised with `mariadb-install-db --auth-root-authentication-method=normal`,
  so `root` can connect over TCP as the Windows install allows. A `mysqld` that
  reports itself as MySQL is refused. `DEVD_MYSQL_ROOT_PASSWORD` is **not applied**
  on these platforms yet (the bootstrap script has no option for it); setup says
  so rather than pretending.
* **PHP** is the one the system has. A package manager installs one version at a
  time, so `runtime install php@8.4` on a machine that has 8.3 fails with that
  explanation. Per-site PHP versions on Linux and macOS need several versions
  installed side by side (for example from Ondřej Surý's PPA or `shivammathur/php`
  on Homebrew); each shows up as the version the system reports.
* **Ports.** An ordinary user cannot bind below 1024 on Linux, and macOS depends
  on the release, so everything but Windows starts on 8080/8443. Sites are served
  at `https://name.test:8443`.

## When Windows has reserved a port

Windows gives blocks of ports to Hyper-V and to the Windows NAT driver (WinNAT)
at boot, and those blocks can cover a port a local stack already uses. MariaDB's
3306 is the usual victim: the server refuses to start with

```text
[ERROR] Can't start server: Bind on TCP/IP port. Got error: 10013
```

while nothing is listening and `devd doctor` says the port is free. A reservation
is not a listener, so neither sees it. `netsh` does:

```powershell
netsh int ipv4 show excludedportrange protocol=tcp
```

`scripts\fix-port-reservation.ps1 -Port 3306` stops and starts WinNAT, which
releases the dynamic reservations until the next boot. It asks for administrator
rights itself. The reservations return after a reboot, often in different
ranges, so the script is something to run again rather than a fix applied once.

## What has been run, and where

This is a record of what was actually executed, not of what is expected to work.

| | Windows | Linux | macOS |
|---|---|---|---|
| Workspace builds and its tests pass | yes, locally and in CI | yes, in CI | yes, in CI |
| Package-manager parsing, verification decisions, generated nginx and Apache files, unit-tested on canned output | yes | yes | yes |
| Install nginx / MariaDB / PHP and start them | yes (downloaded, digest-checked) | **not run**: written, compiled, unit-tested | **not run** |
| PostgreSQL and MySQL installed, started, asked in their own protocol, a database created, dropped and restored from its undo snapshot, stopped cleanly | yes (real servers, local) | adopts the system package; **not run** | adopts Homebrew; **not run** |
| The resident daemon: instance record, attach, stop, service ownership by start time | yes (real processes) | the start-time check reads `/proc`; unit-tested, **not run** as a daemon | the check reads `ps`; **not run** |
| Sign-in start of the daemon | Run value: written; sign-in itself **not exercised** | systemd unit text unit-tested; **not run** | LaunchAgent text unit-tested; **not run** |
| Local DNS server answers over a real UDP socket | yes | the same code; **not run** there | the same code; **not run** there |
| Pointing the operating system at it (`dns.enable`) | commands generated and unit-tested; **never run** (needs administrator rights and port 53) | **not run** | **not run** |
| HTTP/2 in the generated nginx and Apache configuration | the `http2 on;` form (nginx 1.25.1 and later) accepted by a real nginx 1.30.6 and negotiated `h2` over TLS; the older `listen ... http2` form and Apache are unit-tested only | same code; **not run** | same code; **not run** |
| Self-update: signature, checksum and replacement against a local server | yes | yes (unit tests, CI) | yes (unit tests, CI) |
| The window: argument forms, focus and announcements checked in a real browser | yes | n/a | n/a |

Linux and macOS support is written, compiled and unit-tested in CI, and the code
paths that need a real machine (a package manager, a user service manager, a
keychain, the system resolver) have not been run on one. The first run on a real
machine is expected to find things: `cargo test --workspace` there is the first
thing to do, then `devd doctor`.

## Not done

* **Linux and macOS installers.** There is a per-user Linux install script that
  has never been run on Linux, and no macOS package. The Windows installer is
  separate and is exercised on every release.
* **Several PHP versions at once** from one package manager.
* **Code signing.** Neither the installer nor the programs carry an Authenticode
  or Apple signature, so Windows SmartScreen and macOS Gatekeeper warn once. The
  published checksums are signed with devd's own release key, and `devd update`
  verifies them; that protects updates, not the first download.
