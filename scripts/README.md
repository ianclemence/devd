# Cross-check the documentation against the code and the tree.

`check-docs.ps1` answers two questions that are easy to get wrong and hard to
notice:

- **Does every capability the docs name exist?** A backticked id such as
  `git.sync.preview` or `team.accept_suggestion` is a promise. The script collects
  the ids the crates register or dispatch and reports any the docs name that the
  code does not have.
- **Does every relative link resolve?** A `docs/...` link written from inside
  `docs/` resolves to `docs/docs/...` and 404s on GitHub.

```powershell
.\scripts\check-docs.ps1            # report only
.\scripts\check-docs.ps1 -Verbose   # also say how many ids were found
```

It exits 1 when something does not match, so it can run in CI beside the tests.

## What it deliberately ignores

Dotted words appear in the docs that are not capabilities: `devd.yml` keys
(`health.url`), Git configuration (`core.pager`), column names
(`team_tasks.scope`), `php.ini` directives (`opcache.enable`) and code symbols
(`run_ctx.deadline`). Each of those has its own list below, with the reason. When
a new kind of dotted name appears in the docs, add its prefix there rather than
letting the check drift into false alarms.

It also skips any document marked as generated (`docs/stacks.md` is written by a
test). Hand-editing a generated document - even to fix its punctuation - makes
that test fail. Regenerate it instead:

```bash
$env:DEVD_UPDATE_DOCS = "1"; cargo test -p devd-stacks; Remove-Item Env:\DEVD_UPDATE_DOCS
```
