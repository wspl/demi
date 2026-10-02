# Rust database fixtures

`rust-control.sqlite` and `rust-conversations/*.sqlite` were written by
`demi-backend-database`, using its public control and conversation services.
The control database holds the test master account and one conversation. The
conversation database has issued command number 1 and will issue 2 next.

They were written once during the migration by a small Rust program over
the Rust database crate, kept outside the repository in the migration's
reference directory (`gomig-ref/oracles/database/rust-fixture`), since the
repository holds no Rust.

`TestRustDatabasesReadUnchanged` copies both fixtures, opens them with Go,
checks their records and schema versions, and compares the complete file
hashes after closing. Tests do not build or run Rust.
