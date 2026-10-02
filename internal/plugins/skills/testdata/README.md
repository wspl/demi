# Skills manifest fixture

`manifest.json` is the skills entry copied from
`gomig-ref/plugin-manifests/manifests.json`, produced by Rust's registered
plugin manifests. Whitespace is compacted; object member order and values are
preserved. `TestRustManifest` compares the factory's encoded manifest byte for
byte, ignoring only the fixture's final newline.
