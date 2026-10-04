# Browser manifest fixture

`manifest.json` is the browser plugin's expected manifest, including its object
member order and numeric spelling. The test compacts whitespace only and compares the factory's
encoded manifest byte for byte; it does not regenerate expectations from Go.

The d-panel fixtures were refreshed by `gomig-ref/oracles/d-panel/src/main.rs` against the current reference.

The LiveErrorCode description uses the owner-approved Go wording
(`capture_unavailable` or `capture_failed`, decision 2026-10-04) instead of
the reference fixture’s bracketed constant links. Schema titles also use the approved Go type spellings from i-renames; the
remaining fixture content is unchanged.
