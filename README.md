# myschema

Diffs two live MySQL schemas and generates the SQL to reconcile them — a
MySQL equivalent of this workspace's own `pgdiff` (which does the same
for Postgres), and a `migra` (Python) equivalent for people who don't
want a Python install just to compare two schemas.

## Installation

```sh
cargo install --path .
```

## Usage

```bash
myschema mysql://user:pass@host1/db1 mysql://user:pass@host2/db2
```

Reads both schemas from `information_schema.columns`, prints every
difference as a ready-to-run DDL statement bringing the first database's
schema forward to match the second's.

## Status: built and verified with a full real round trip — generate, apply, re-diff to zero — against a real MySQL, including two real bugs caught along the way

- **11 unit tests** (`cargo test --lib`): every change category (added/
  removed table, added/removed column, column type change, nullability
  change), identical schemas producing no changes, and SQL generation
  for each category including the nullability-change fix below as its
  own locked-in regression test.
- **A real bug caught live against a real MySQL 8 server**: the query
  against `information_schema.columns` initially decoded `TABLE_NAME`/
  `COLUMN_NAME`/etc. as typed `String` values, which panicked with
  `mismatched types; ... VARCHAR ... is not compatible with SQL type
  VARBINARY` — on this real server, those columns come back as
  VARBINARY, not VARCHAR, and `sqlx`'s typed decode rejected them
  outright. Fixed by explicitly `CAST(... AS CHAR)`-ing every text
  column in the query itself, forcing a real VARCHAR result.
- **A second, more serious real bug caught live, this time via an actual
  syntax error from a real server**: the nullability-change SQL
  generator originally produced `ALTER TABLE users MODIFY COLUMN email
  NOT NULL;` — real MySQL's `MODIFY COLUMN` requires the *full* column
  definition (type included), not just the nullability keyword, and
  running this exact statement against a real database returned `ERROR
  1064 (42000): You have an error in your SQL syntax`. This would have
  meant the tool's own generated SQL was unusable for exactly the
  category of change it exists to automate. Fixed by threading the
  column's real data type through to the nullability-change case, with
  a dedicated regression test locking in the fix.
- **Full real round-trip verified end to end, exactly like `pgdiff`'s
  own bar**: built two real, deliberately different MySQL schemas in a
  scratch MySQL 8 container (a removed table, an added table, a column
  type widened from `int` to `bigint`, a column made `NOT NULL`, a new
  nullable column) — generated the reconciling SQL, applied that
  **exact, unmodified** output to the first database via a real `mysql`
  client, and re-ran the diff: **zero differences**, confirming the tool's
  own output is genuinely correct SQL, not just plausible-looking text.

**Not done / deliberately deferred**: indexes, primary/unique/foreign
key constraints, and default values aren't compared, only column
presence/type/nullability — matching this workspace's own `pgdiff` v1
scope, which defers the identical set of constraint types for the same
reason (column-level diffing covers the overwhelming majority of real
day-to-day schema drift). Character set/collation differences on a
column aren't detected, only the base `COLUMN_TYPE` value.
