---
name: database-migrations
description: Change a schema without losing data or locking the database
kind: build
roles: data,backend
---

# Database migrations

Data outlives code, so every migration is treated as if it cannot be undone. Additive first, repeatable, tested on empty and populated copies.

## When to use

Any change to tables, columns, indexes or constraints, and any backfill.

## Procedure

1. **Additive first.** Add the new column, table or index. Do not drop or rename in the same step.
2. **Expand, move, contract.** Add the new shape; write both and read the new; backfill; and only in a later change remove the old shape.
3. **Backfill in batches** so no statement holds a long lock on a large table. A new `NOT NULL` column needs a default or a backfill first.
4. **Repeatable.** A migration run twice, or after a partial failure, does no harm (`make-it-safe-to-rerun`).
5. **Constrain in the schema.** Types, `NOT NULL`, `UNIQUE` and foreign keys say what must be true.
6. **Indexes for queries that exist.** Check the plan of the query the index is for.
7. **Test twice:** from an empty database to the latest version, and from a copy of realistic data.

## Pitfalls

- Dropping, truncating or rewriting existing data without the person's explicit approval. Say what would be lost.
- Testing only on an empty database.
- Running against the live database to "see what happens". Use a copy (`touch-only-what-you-started`).

## Verification

Both test runs are reported, and a second run of the migration is shown to change nothing.
