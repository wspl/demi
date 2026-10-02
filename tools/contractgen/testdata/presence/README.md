# Field presence fixtures

`Defaults` covers Rust `#[serde(default)]` without `skip_serializing_if`:
absence supplies empty collections, false, zero or empty text; serialization
still writes the field. `crates/plugin-interface/src/manifest.rs` uses this
for Manifest's commands, profiles, context and streams and for the lists in
Stream, Page, State and Method. Present null is not a default.

Schemars 1.2.2 `schemars_derive/src/schema_exprs.rs::expr_for_struct` inserts
`Default::default()` as the property's `default` and excludes the property
from `required` for the deserialize contract. The schema assertion in
`TestDefaultFields` checks those exact annotations, and compiles the schema
against the same absent, empty, populated and invalid inputs as the codecs.
`crates/xtask/src/contracts/zod.rs::fields` renders non-required properties
with `.optional()`, without inserting defaults; `verify.mjs` checks this with
actual Zod. The captured Rust files in gomig-ref/generated use that same
optional-property convention. Those production outputs are unaffected by
this fixture.

`Labels` is named to exercise default collection handling through generated
methods as well as the plain Items slice. Encoding a zero Go value writes
empty collections without changing the caller's nil fields.
