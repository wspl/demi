# Browser manifest fixture

`manifest.json` is the browser object copied from
`gomig-ref/plugin-manifests/manifests.json`, produced by Rust's registered
plugin manifests. Its original object member order and numeric spelling are
preserved. The test compacts whitespace only and compares the factory's
encoded manifest byte for byte; it does not regenerate expectations from Go.
