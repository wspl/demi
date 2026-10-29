---
status: proposed
---

# Each migration stands alone, and a test runs every one

## Problem

Alex adds a column to the control database. They add the schema `main`
held to the control history, with a migration, in
`crates/backend-database/src/schema.rs`. That file is 6,600 lines: the
current schemas, 80 earlier ones by name, and their migrations, in no
order: the migration from 0.1.11 is at line 4,182, the one from 0.1.65 at
line 394, and the SQL of a step and the Rust it calls sit apart.

Some migrations call the code that serves today: one makes its tables from
today's control schema, two run the usage ledger's own passes, others
decode a schedule, a quota snapshot, a model catalog or a fork's record as
today's types and write it back, and one makes a Cloud device through
`devices.rs`, which keeps a column list of an earlier schema for it. When
that code changes, what an old migration does changes with it, unseen: the
step from 0.1.5614 makes its tables from the current schema, so the next
change to one of them changes what that step leaves, and the step after it
fails on a server that upgrades from before both.

The history of the oldest conversation databases, whose rows held whole
blocks, is worse: its 30 steps and the conversion that follows them
decode every block and checkpoint as today's types. No database of that
form is left on any server.

No test runs a migration. A mistake shows on the first server that
upgrades. Running every step on an empty database of its earlier schema,
as the spike below did, shows one already: a control database migrated
from any schema before 0.1.5565 gives `push_subscriptions` a default its
new databases lack.

## Decision

Nothing a user sees changes. Every schema's text, and so its version,
stays as it is, but for the one fix below.

### One file per step

```text
crates/backend-database/src/schema/
  mod.rs                    Schema, Shipped, Migration, version_of, apply
  control.sql               the current control schema
  conversation.sql          the current conversation schema
  control/
    mod.rs                  HISTORY: the steps, oldest first, one line each
    from_0_1_11.rs          what the step changes, and its migration
    from_0_1_11.sql         the schema it starts from
    ...
  conversation/             the same
```

A step's file says in its entry comment what the step changes, which its
migration's own comment says today; the list in `CONTROL`'s comment goes. The history in
`mod.rs` names each step once, in order:

```rust
pub(super) const HISTORY: &[Shipped] = &[
    Shipped { sql: include_str!("from_0_1_11.sql"), migration: from_0_1_11::MIGRATION },
    // ...
];
```

### A step depends on nothing that changes

A step uses `rusqlite`, `serde_json`, and what the migration is handed:
the blobs a conversation's values name (`ReadBlob`) and the vault that
seals again what it sealed (`ResealSealed`). It never uses a type or a
function the rest of the crate or another crate serves with. A step that
needs a stored value's shape keeps a copy of that shape as it was when the
step was written, or works on the JSON. What several steps of one history
use is in a module of that history beside them, which serves only them.
`schema/mod.rs`'s entry comment states this rule.

The control steps that use today's code get their own:

- The step from 0.1.5614 takes its tables from 0.1.5658's schema, the one
  it leads to.
- The usage ledger's two passes, of the steps from 0.1.37 and from
  0.1.5586, are copied as they are today into `control/ledger.rs`.
- The schedules', quota snapshots', model catalogs' and forks' records are
  rewritten as JSON, in the shape the step wrote, and not decoded as
  today's types.
- The Cloud device of the step from 0.1.22-dev.20261016T043704Z is made by
  the step itself, and `devices.rs` loses the column list it kept for it.

### The oldest conversation databases are no longer read

The history of the conversation databases whose rows held whole blocks,
and the conversion of such a database into a new file (`convert.rs`), are
removed, since no server holds one. A database of that form is refused as
one of any version outside the history is: `demi-server` names it and stops
the upgrade before anything changes. With the conversion go its hard links
in an upgrade's snapshot, so every database the snapshot keeps is a copy.

### A test runs every step

One test binary, `crates/backend-database/tests/backend-database`, takes
each database's history and, for every step:

1. Makes an empty database of the step's schema, and records its version.
2. Migrates it to the current schema, as an open does.
3. Compares it with a new database of the current schema: the same
   tables, each with the same columns, types, `NOT NULL`, defaults, keys,
   foreign keys and `STRICT`; the same indexes, with their columns,
   uniqueness and `WHERE`; the same triggers and views.

A step written in Rust from now on also has a test of its own there: rows
of its schema in, the rows it must leave out. The existing ones have none:
no server holds the data they were written for, so no test could say what
they must leave.

### What the test found

A new step, from 0.1.5672's schema, makes `push_subscriptions` anew
without the default, so every control database matches a new one. The
current schema's text gains a comment that says the column has no default,
which gives it its new version.

## Spike

A temporary test ran every step of the three histories on an empty
database of its earlier schema and compared the result with a new
database of the current one:

- Conversation (1 step) and the oldest conversation databases' (30 steps):
  all match.
- Control, 50 steps: the 47 from before 0.1.5565 give
  `push_subscriptions.quiet_while_elsewhere` the default `1`, which the
  column's migration needed to add it `NOT NULL`, and a new database's
  column has no default. An insert that omits the column succeeds on an
  upgraded server and fails on a new one.

The test, with the step that fixes the default left out, fails on those 47
steps. The comparison could not read `CHECK` constraints: SQLite reports them
only in each table's SQL, whose text differs between a table a migration
altered and one made new. The test leaves them out, and so the collation
of a column no index covers, which only that text holds too.

## Prior art

- [Django](https://docs.djangoproject.com/en/stable/topics/migrations/#historical-models)
  hands a data migration the historical version of each model, never the
  one the code imports, so that a migration keeps meaning what it meant;
  its test runner builds each test database by running every migration.
- The [Rails guides](https://guides.rubyonrails.org/active_record_migrations.html)
  warn against using an application model in a migration, since the model
  will change after it, and define a minimal copy in the migration
  instead.
- [refinery](https://github.com/rust-db/refinery) and
  [sqlx](https://github.com/launchbadge/sqlx/tree/main/sqlx-macros-core/src/migrate.rs)
  keep one file per migration, ordered by its number.

Demi keeps its version as the digest of the schema's text rather than a
step's number, so the history stays a list of the schemas themselves, and
it adds the comparison with a new database, which none of them makes.

## Alternatives

- Leave the steps in `schema.rs` and add only the test: the test finds what
  a step does wrong, but a step that uses today's code still changes when
  that code does.
- A build script that lists the step files: the list in `mod.rs` is one
  line per step, and the compiler already refuses a step it names that is
  missing.

## Open questions

None.
