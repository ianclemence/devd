---
name: database-migrations
description: Change a schema without losing data or locking the database: additive first, repeatable, tested on empty and populated copies.
kind: playbook
roles: data,backend
---

# Database migrations

Data outlives code. Treat every migration as if it cannot be undone.

1. **Additive first.** Add the new column, table or index. Do not drop or rename in the same step.
2. **Expand, move, contract.** Add the new shape, change the code to write both and read the new, backfill existing rows, and only in a later change remove the old shape.
3. **Backfill in batches** so no single statement holds a long lock on a large table. A new `NOT NULL` column needs a default or a backfill first.
4. **Repeatable.** A migration run twice, or after a partial failure, does no harm: `IF NOT EXISTS`, guarded changes.
5. **Constrain in the schema.** Types, `NOT NULL`, `UNIQUE` and foreign keys say what must be true; code does not have to remember it.
6. **Indexes for queries that exist.** Check the plan of the query the index is for.
7. **Test twice.** From an empty database to the latest version, and from a copy of realistic data. Report both.
8. **Never** drop, truncate or rewrite existing data without the person's explicit approval. Say what would be lost.
