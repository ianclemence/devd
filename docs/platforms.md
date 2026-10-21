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

## What has been verified, and what has not

| | State |
|---|---|
| Windows | `cargo test --workspace` passes |
| Linux, macOS compile | `scripts/cross-check.ps1` type-checks the whole workspace (tests included) for `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` from a Windows machine. Nothing is linked or run |
| Package-manager parsing, verification decisions, argument lists for `security`, the generated nginx files | unit-tested on canned output, on every platform |
| Linking into the layout, `install_system` end to end | tests exist (`cfg(unix)`) and compile for Linux and macOS; **they have not been run** - there was no Linux or macOS machine |
| Starting nginx / MariaDB / php-cgi as an ordinary user, `mariadb-install-db` against a real package, `security` against a real keychain | **never executed** |

Treat Linux and macOS support as written, compiled and unit-tested, not as proven.
The first run on a real machine is expected to find things; `cargo test
--workspace` there is the first thing to do, and `docs/research/herd-parity/`
lists what else was left unverified.

## Not done yet

* **Resolving `*.test`.** On Linux and macOS the hosts file is the only mechanism;
  there is no wildcard and no resolver. A loopback resolver (dnsmasq,
  `/etc/resolver`) is milestone M3 in the roadmap.
* **Installers.** There is a per-user Linux install script (never executed on
  Linux) and no macOS package.
* **Several PHP versions at once** from one package manager.
