# Runner MessagePack oracle

These eight files are copied unchanged from
`crates/runner-protocol/tests/fixtures`, preserving their direction and names.
Rust records the frames using `rmp_serde::to_vec_named` in
`crates/runner-protocol/src/wire/mod.rs`.

`TestRunnerMessagePackCorpus` extracts the raw embedded commandwire values
and checks their generated decode/encode round trips byte for byte. Contexts
cover agent and user callers, locales and language tags; package descriptors
cover executable artifacts and sorted target maps. The corpus has no populated
package resources, so those generated codecs have no Rust fixture value to pin.
The manifest MessagePack fixture has an empty package map; the error artifact
location and minimal job exit contain no location or edits to extract.
