# What carries over from other development stacks

devd does not insist on being the only way to run your machine. This page says
what it will use as it is, what it will replace only with your say-so, and what
it will not do at all. Nothing on this page happens on its own.

## It adopts what you already run

| What | What devd does |
|---|---|
| **Apache or nginx** already installed | `service.adopt` records the program you have and uses it. devd asks the package manager which package owns the file and whether it is still what was installed, and refuses one that differs. |
| **MariaDB or MySQL** already running | Detected and reported, never assumed healthy. `services.decide` records whether devd adopts it, watches it only, treats it as an external instance on a port, or ignores it. |
| **Redis, PostgreSQL, Docker** | The same: detected, shown with the state it reports, and left alone unless you decide otherwise. |
| **PHP** installed outside devd | Reported, not silently swapped. A site is served through a FastCGI listener devd runs, which is built from a runtime devd downloaded and verified; adopting a foreign PHP is a separate, explicit decision. |
| **A folder of projects** | `site.park` makes a site for each web project in it. Nothing is written into the folders. |
| **An existing database** | `db.copy` brings a database over from another MySQL or MariaDB instance into devd's own, with one command, and says what was and was not carried over. |
| **Your shell session** | `autostart enable` starts the stack when you sign in, using what each platform gives an account without administrator rights. |

## What it moves you away from, with your approval

Replacing an installation is never a side effect of something else. It is its
own operation, it is destructive, and it asks first:

- installing a service where one already answers leaves yours alone and reports
  the conflict;
- removing anything devd installed keeps the data directory unless you say
  otherwise, and saves a snapshot before it deletes anything;
- switching to a version of PHP, Node.js or Go only changes what devd serves,
  not your `PATH`, unless you run `runtime.use`.

## What it does not do

- **It does not deploy.** There is no deploy capability and no path through an
  agent to production.
- **It does not open ports** to another machine. The window, the HTTP API and the
  MCP server are loopback only.
- **It does not edit the hosts file on its own.** `devd hosts sync` does, and
  asks first; a declined plan never runs.
- **It does not take a port from a program it did not start.** A stranger on
  MariaDB's port is reported as `ambiguous_owner` and left running.
- **It does not promise more on Linux and macOS than has been verified.** See
  [`platforms.md`](platforms.md), which records exactly what has and has not been
  run on a real machine.

## What does not carry over yet

- A tool that is installed but reports no version is shown as installed, with
  the reason; devd does not guess a version and does not treat it as healthy.
- A service started outside devd is not health-checked on the same schedule as
  devd's own services. It is watched only if you asked for that.
- Settings that live inside another stack's own configuration file (an Apache
  `httpd.conf` you keep by hand, for example) are read, never rewritten.
