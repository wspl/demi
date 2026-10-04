# Browser contract fixtures

`types.go` holds browser command shapes the generator must handle
(`internal/commandpackage/browser/browserproto` owns the real ones, and the browser
plugin's manifest golden pins their schemas): `NodeValue` is a number or
string union, `Wheel` carries the two negative ranges of the live view's wheel
input, `Scalar` exercises overlapping numeric variants and boolean and object
variants, `DialogInspectResult` holds a nullable object, `ErrorDetails` is a
tolerant object with an optional flattened `AssetsExportResult`, and
`Optional` checks an optional flattened object between sibling properties.

`nullable-manifest.json` keeps every nullable schema occurrence of the
built-in plugins' manifests, keyed by its JSON pointer: plain strings,
date-time and bounded strings, objects, and references.

An optional flattened object decodes as absent when it is absent, incomplete
or malformed: the decoder tries the child and drops it on any error, while the
parent still checks its own fields.

`TestBrowserScalarVariants`, `TestBrowserOptionalFlatten`,
`TestBrowserNegativeBoundsAndNullable`, `TestBrowserScalarNumberBytes` and
`TestManifestNullableShapes` use these fixtures. `verify.mjs` runs the
generated Zod against the browser examples and the mixed scalar union;
`TestBrowserTypeScript` runs it with the `acceptance` build tag after `npm ci`
in the parent `testdata` directory. No server, browser, model or network is
used.
