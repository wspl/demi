# Rust vault fixtures

`rust-sealed.txt` contains two records written by the unchanged
`crates/backend-providers/src/vault/seal.rs`, called from a small standalone Rust program
run once during the migration and kept outside the repository, in the
migration's reference directory (`gomig-ref/oracles/b-providers/rust-seal`),
since the repository holds no Rust. Its ID aliases only replace identifier construction; the
reference encryption, row naming, nonce generation and layout are used directly.
The program also opens each record with Rust before writing it.

Both records use a 32-byte key filled with `07`, provider `entry-1`, and (for
`secret`) account `cred-1`. The configuration plaintext is
`{"apiKey":"sk-test-123"}`; the account plaintext is
`{"refresh":"rust-token"}`. These are synthetic test credentials.

Random nonces make regeneration change the ciphertext. Ordinary Go tests read
only the committed fixture and never invoke Rust. `TestSealedValueOpensOnlyForItsRowUnderItsKey`
opens both Rust-written records and checks wrong keys, copied rows, tampering,
truncation and ambiguous identifier concatenations.
