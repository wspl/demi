The 59 backend-to-runner frames, 56 runner-to-backend frames, kept-output
records, and manifest.json are copied verbatim from
crates/runner-protocol/tests/fixtures.

TestDirectionalCorpus checks all 115 frames byte for byte. TestKeptCorpus
checks the stored records. TestManifestBuild checks the built manifest
against manifest.json and its recorded Rust canonical digest.

The opaque manifest in backend-to-runner/manifest.msgpack is a transport
fixture, not a valid command manifest: its hash is 64 `e` characters and its
`demi` group has no subcommands. Wire decoding round-trips it; manifest
verification refuses it, just as Rust separates these two boundaries.
