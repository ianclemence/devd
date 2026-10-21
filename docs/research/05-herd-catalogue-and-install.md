# How Herd fetches PHP versions and installs its services — and what devd should do

Herd 2.0.2, installed on this machine at `C:\Program Files\Herd`.

## What was read

The Electron main process is shipped as **V8 bytecode** (`out/main/index.jsc`,
2.4 MB), not as source, so the code could not be read. What could be read is
the string constants, which is where the endpoints, the headers and the
messages live. The PHP backend (`resources/bin/herd.phar`) is readable PHP
and was read where it answered the question.

Everything below is an observation, not an inference, unless it says so.

## The catalogue

Herd does not read each vendor's own listing. It reads **its own catalogue**,
one endpoint per platform:

```
GET https://herdphp.com/api/services/win
```

```json
[{"type":"mysql","label":"MySQL","version":"9.7.2","default_port":3306,
  "categories":["database"],
  "download_path":"https://download.herdphp.com/services/mysql/9.7.2-win.zip",
  "is_available":1}, ...]
```

Eight services, each with its type, its version, **the port it uses**, its
categories, and a download path on a CDN. The list is the whole answer to
"what can I install, at what version, on what port, from where".

The requests carry the machine's identity, so the server can answer with the
right build:

```
Accept
X-Herd-Version
X-OS-Version
X-Herd-Device-ID
X-Herd-License-Key
X-Herd-Platform
```

## PHP versions

PHP is different: Herd asks **per series** and is told the latest patch and
whether it is behind:

```
GET https://herdphp.com/api/php-update/8.4/win/
{"version":"8.4.26","internalVersion":26,"updateAvailable":true}
```

Herd knows the *series* it supports (`Valet\Herd::SUPPORTED_PHP_VERSIONS`:
7.4, 8.0, 8.1 …) and asks the catalogue for each one. The patch level is
never written down by Herd; it is resolved at the moment of the question, and
`updateAvailable` is the answer to "should I tell the person".

The application itself has a second set of URLs — `api/windows/update`,
`api/windows/versions`, `api/licenses`, `api/resource-update/` — but those are
the **application's** own update feed, not the PHP catalogue.

## Digests

The main process carries the strings `sha256`, `expectedChecksum`,
`verifyChecksum`, `checksum`, and this:

> ` was published without a checksum, not installing`

So Herd's rule is: **no checksum, no install.** The archive is deleted and
nothing is installed when the digest does not match, and an artefact that
arrives without a published digest is refused outright.

## What devd does today

| | Herd | devd |
|---|---|---|
| Service catalogue | one endpoint, one shape | nginx.org (an **HTML page**) for nginx; `downloads.mariadb.org/rest-api` for MariaDB |
| PHP versions | one endpoint per series | windows.php.net's own listing, per series |
| Digest | required | required for PHP and MariaDB; **nginx has none** |
| Port | in the catalogue | in the service definition |
| Platform | a header, so the server answers with the right build | the catalogue is filtered locally |

devd's `Available` already carries what Herd's catalogue carries and more:
the exact version, the series, the channel, the download URL, the digest, the
size, the publication date and the executable's name. The shape is not the
problem.

## What to change

### 1. nginx must be verified

nginx.org publishes a **PGP signature** (`nginx-1.31.7.zip.asc`) and no
checksum. devd therefore installs nginx **unverified today** — the download
is checked for size, and that is all.

Herd's rule says: do not install what you cannot verify. devd should not
either. Two ways:

- verify the PGP signature against nginx's published key — correct, and a
  large dependency;
- **record the digest when the archive is first fetched, and check every
  later download against it** — what devd now does.

The second is what this repository now does.
`crates/devd-services/defs/nginx-digests.txt` holds the digests of the
archives devd offers, and `catalogue::nginx` puts them in the catalogue, so
the existing download path verifies the bytes. An archive that does not match
is deleted and nothing is installed. An nginx version with no recorded digest
is **refused**: devd will not install a program it cannot check.

### 2. One catalogue, not two readings

devd reads nginx from an HTML page. That is the weakest link in the chain: a
markup change on the vendor's site changes what devd can install, and an
HTML page is not a contract. nginx.org's own directory listing
(`https://nginx.org/download/`) is the machine-readable form of the same
thing, and the digests above pin the result.

The deeper point is the one Herd's design makes obvious: **a catalogue is
data, and the installer should not know where it came from.** devd already
has the right shape (`Available`). What is missing is one reader per vendor
that fills it in, so that the service definitions, the install path and the
screen all speak about the same thing.

### 3. The series/patch distinction is already right

Herd asks per series and is told the patch. devd's runtime catalogue does the
same with `series_of` and `version_key`. The one thing worth copying is the
*shape of the answer*: `{"version": "8.4.26", "updateAvailable": true}` says
both "here is the latest" and "you are behind" in one call, so a check and an
install cannot disagree.

### 4. The port belongs with the service

Herd's catalogue carries `default_port` next to the version. devd carries it
in the service definition, which is the same information in the same place —
the service is the thing that has a port, not the catalogue entry.

### 5. What devd should not copy

- **A first-party catalogue of PHP versions.** Herd reads PHP from
  `herdphp.com`, Laravel's own service. devd reads it from windows.php.net,
  the PHP project's own site, which is where the digests live. A first-party
  catalogue is a good idea for services nobody publishes binaries for; it is
  a bad idea for runtimes somebody does.
- **A licence key in the request.** devd has nothing to gate and nothing to
  report.
- **Parsing an HTML page for versions.** See above.

## Evidence

- The endpoint, its JSON shape and its headers: the string constants of
  `out/main/index.jsc`.
- The PHP answer, live:
  `GET https://herdphp.com/api/php-update/8.4/win/` →
  `{"version":"8.4.26","internalVersion":26,"updateAvailable":true}`.
- The service catalogue, live: `GET https://herdphp.com/api/services/win` →
  eight services with `type`, `label`, `version`, `default_port`,
  `categories`, `download_path` and `is_available`.
- The checksum rule, in the same string table.
- `Valet\Herd::SUPPORTED_PHP_VERSIONS` and the per-version configuration in
  `resources/bin/herd.phar` (`cli/Valet/Herd.php`, `config/php.json`).
