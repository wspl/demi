The `rust/` directory was written by `make_fixture.rs`, linked against the judge
Rust tree's rusqlite, rusqlite_migration, argon2, demi_core, demi_agent, and
demi_runner_protocol libraries. It calls the
original backend's `schema::CONTROL` and `schema::CONVERSATION` migrations.
The scratch `schema_body.rs` is the original `crates/backend/src/storage/schema.rs`
with its leading module documentation removed for `include!`; no SQL is changed.

The password is the made-up string `fixture password`. No real credential is
present. The PHC string is Rust Argon2's output, not a Go-generated hash.
Go tests copy this directory before opening it, so the committed fixture stays
unchanged and requires no Rust compiler during an ordinary test run.

All 25 tables contain representative rows (26 rows in total). `rows.json` is
the Rust reader's snapshot of every column, including blobs represented as hex.
The checkpoint, user block, and command output also use their Rust serializers.
The fixture test compares the snapshot and exercises the typed Go readers.
