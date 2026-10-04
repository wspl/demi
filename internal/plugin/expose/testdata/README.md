# Expose manifest

`manifest.json` is the expose plugin's registration manifest.
`TestManifestMatchesGolden` compares the public factory's complete encoded
manifest with these compact bytes, retaining every keyword and property
position. No Go-generated expected values or schema normalization are used.
