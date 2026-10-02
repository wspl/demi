# Rust database fixtures

`rust-control.sqlite` and `rust-conversations/*.sqlite` were written by
`demi-backend-database`, using its public control and conversation services.
The control database holds the test master account and one conversation. The
conversation database has issued command number 1 and will issue 2 next.

The generator is `rust-fixture/src/main.rs`; its dependencies point to this
checkout's Rust reference. To reproduce into a new, empty directory:

```sh
cargo run --locked --manifest-path internal/backend/database/testdata/rust-fixture/Cargo.toml --target-dir /tmp/b-database-rust-target -- /path/to/empty-directory
```

`TestRustDatabasesReadUnchanged` copies both fixtures, opens them with Go,
checks their records and schema versions, and compares the complete file
hashes after closing. Tests do not build or run Rust.
