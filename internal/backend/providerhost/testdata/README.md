# Vault fixtures

`sealed.txt` holds two records in the stored credential format that existing
databases contain: AES-256-GCM under the vault key, a 12-byte nonce prefix,
and the row name (`demi provider config` or `demi account secret`, then
each id as a 4-byte big-endian length and its bytes) as additional data.

Both records use a 32-byte key filled with `07`, provider `entry-1`, and (for
`secret`) account `cred-1`. The configuration plaintext is
`{"apiKey":"sk-test-123"}`; the account plaintext is
`{"refresh":"rust-token"}`. These are synthetic test credentials.

Random nonces make regeneration change the ciphertext, so the records are
never regenerated. `TestSealedValueOpensOnlyForItsRowUnderItsKey` opens both
records and checks wrong keys, copied rows, tampering, truncation and
ambiguous identifier concatenations.
