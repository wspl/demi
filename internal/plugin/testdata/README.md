# Rust plugin manifests

`manifests.json` is copied byte for byte from the migration reference at
`gomig-ref/plugin-manifests/manifests.json`. It contains all built-in plugin
manifests emitted by the Rust reference. `TestRustManifests` decodes through
the generated contract and compares the complete re-encoded, pretty-printed
file, preserving field and embedded schema key order.
