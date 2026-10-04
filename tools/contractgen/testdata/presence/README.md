# Field presence fixtures

`Defaults` covers a field that has a default when absent but is always
written: absence supplies an empty collection, false, zero or empty text, and
encoding still writes the field. Present null is not a default. The plugin
manifest uses this for its commands, profiles, context and streams, and for the
lists in Stream, Page, State and Method.

In the JSON Schema such a field carries its default as `default` and is left
out of `required`. `TestDefaultFields` checks those exact annotations and
compiles the schema against the same absent, empty, populated and invalid
inputs as the codecs. The generated Zod renders a non-required property with
`.optional()` and inserts no default; `verify.mjs` checks this with Zod
(`TestNullableOptionalTypeScript`, `acceptance` build tag).

`Labels` exercises default collection handling through generated methods as
well as the plain Items slice. Encoding a zero Go value writes empty
collections without changing the caller's nil fields.
