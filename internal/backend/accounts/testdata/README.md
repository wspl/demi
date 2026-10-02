# Accounts reference fixtures

The standalone Rust fixture writer lives outside the repository at
`/Users/zan/Projects/demi-worktrees/gomig-ref/oracles/b-accounts/reference`.
It uses the reference backend's Argon2 and ICU versions and is not part of a
Go test or build.
Run from the repository root:

```sh
cargo run --offline --manifest-path /Users/zan/Projects/demi-worktrees/gomig-ref/oracles/b-accounts/reference/Cargo.toml -- "$PWD/internal/backend/accounts/testdata"
```

It writes `passwords.tsv` (password, base64 salt, PHC) and `locales.tsv`
(input tag, ICU canonical tag or `ERROR`). It also writes the production
`../iana_names.txt`, every accepted normalized zone spelling from ICU's
`IanaParserExtended::iter_all`, excluding unknown zones. Aliases retain their
normalized spelling, not the canonical zone they refer to.

`go-password.tsv` contains a password, raw salt and PHC written by Go's
`hashPassword`. The Rust fixture writer verifies it with `Argon2::default()`;
`TestGoPasswordFixtureVerifiedByRust` checks that Go still writes those bytes.
`TestRustPasswordFixtures` verifies the Rust hashes in Go and checks that Go
writes identical PHC bytes for their salts. All passwords here are public
fixture text, not credentials.

These TSV records are fixture transport, not application JSON contracts.
