# Setup: what devd needs, and who installs it

A machine is not a clean slate. devd needs a few things to do its job, and the
first run should say what is missing rather than failing later with a stack
trace or an empty page.

There is one list, computed by one capability, and every surface shows it:

```
setup.requirements
```

Each entry says what the thing is, why devd wants it, whether devd can work
without it, its current state (`ready`, `missing` or `broken`), and the fix:
the capability to call with its arguments, the view to open, or the exact
command to run by hand.

## What is on the list

| Requirement | Why devd wants it | How it is fixed |
|---|---|---|
| Git | every repository feature, and the isolated copies the agent team works in | `toolchain.install` |
| PHP | sites are served by PHP | `runtime.install` |
| nginx | the web server that serves sites over HTTP and HTTPS | `service.install` |
| MariaDB | the local database devd offers | `service.install` |
| The PHP listener | runs PHP for your sites | `service.install` |
| A model provider and its key | the built-in assistant and the agent team need a model | the Models page |
| The WebView2 runtime | the native window is drawn by it; the command line does not need it | a download, with the exact command shown |

Nothing on this list is installed by the capability itself. It is a read-only
capability: it reports. Every fix is an ordinary capability call, so it goes
through the permission model like anything else that changes the machine.

## Where it is asked for

**At install time.** The Windows installer checks for the WebView2 runtime
before it finishes, because without it the devd window cannot open at all. It
asks first, and installs it only if the answer is yes. Everything else is
installed by `devd setup`, which the installer runs on first launch.

**In the command line.** `devd setup` prints the whole list before it changes
anything, saying which items this run will install and which need you:

```
==> Requirements
    A model provider — The built-in assistant and the agent team need a model.
      open providers in the app to finish this
==> Runtime
    PHP is already installed
```

**In the window.** The home page has a "Needs installing" panel with one action
per item, and Diagnostics has the full list with each item's state. The action
is the capability call, so the window shows the plan and asks for approval the
way it does for anything else.

**In the terminal.** The dashboard has a Setup section listing what is missing,
and `i` installs the first thing it can. The toolchain view still lists every
tool with its own install and remove keys.

## Rules

* Nothing is installed silently. A missing thing is offered, with the reason,
  and installing it is a decision the person makes.
* A missing optional thing never makes devd look broken. The model provider is
  needed by the assistant, not by the file system, the web server or Git.
* A thing devd cannot install itself says so, and gives the command to run.
* The list is the same everywhere. The installer, the command line, the window
  and the terminal never disagree about what is missing.
