# Rust expose manifest

`manifest.json` is the unchanged expose object extracted from
`gomig-ref/plugin-manifests/manifests.json`, captured from the Rust factory.
Only the surrounding array and other plugins are omitted. No Go-generated
expected values or schema normalization are used.

`TestRustManifest` compares the public Go factory's complete encoded manifest
with these compact bytes, retaining every keyword and property position.
