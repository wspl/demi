`rust-vault.json` was written by `make_vault_fixture.rs`, linked against the
judge Rust tree's aes-gcm, hkdf, sha2, hex, rand, thiserror, tokio,
demi_web_api and demi_artifact libraries. The program includes the original
backend's `crates/backend/src/vault/seal.rs` and `secret.rs` unchanged, less
their leading module documentation (`include!` refuses it), as the scratch
files `seal_body.rs` and `secret_body.rs`. So the sealed values are the Rust
vault's own output, which the Go vault must open.

Every value is made up: the instance secret is the bytes 0 to 31, and the key
and token are fixture strings. No real credential is present.
