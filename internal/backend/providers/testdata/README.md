# Rust vault fixtures

`rust-sealed.txt` contains two records written by the unchanged
`crates/backend-providers/src/vault/seal.rs`, called from the standalone
`rust-seal` program. Its ID aliases only replace identifier construction; the
reference encryption, row naming, nonce generation and layout are used directly.
The program also opens each record with Rust before writing it.

Both records use a 32-byte key filled with `07`, provider `entry-1`, and (for
`secret`) account `cred-1`. The configuration plaintext is
`{"apiKey":"sk-test-123"}`; the account plaintext is
`{"refresh":"rust-token"}`. These are synthetic test credentials.

Regenerate from the repository root before the Rust reference is removed:

```sh
CARGO_TARGET_DIR=/tmp/b-providers-rust-seal-target cargo run \
  --manifest-path internal/backend/providers/testdata/rust-seal/Cargo.toml \
  --offline > internal/backend/providers/testdata/rust-sealed.txt
```

Random nonces make regeneration change the ciphertext. Ordinary Go tests read
only the committed fixture and never invoke Rust. `TestSealedValueOpensOnlyForItsRowUnderItsKey`
opens both Rust-written records and checks wrong keys, copied rows, tampering,
truncation and ambiguous identifier concatenations.
