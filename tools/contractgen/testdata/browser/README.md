# Browser contract fixtures

`types.go` holds browser command shapes the generator must handle
(`internal/cmdpkg/browser/browserop` owns the real ones): `Wheel` carries the
two negative ranges of the live view's wheel input; `Scalar` exercises
overlapping numeric variants and boolean and object variants; `Optional`
checks an optional flattened object between sibling properties.

The schema snapshots are taken unchanged from the built-in plugins' captured
manifests (`../schemas/manifests.json`):

- `NodeValue`: browser `inspect` result, `tree.items.properties.value`.
- `ErrorDetails` and `BrowserFailure`: browser `content.fetch` result,
  `pages.items.properties.error` and its `details` property.
- `DialogInspectResult`: browser `dialog.inspect` result, whole schema.

Nested snapshots add only a root `title`, as generated schema roots do.
`nullable-manifest.json` keeps every nullable schema occurrence in that
manifest, keyed by its JSON pointer: plain strings, date-time and bounded
strings, objects, and references. `failure.json` is the export-details
document a browser failure prints, for tab `t1`.

An optional flattened object decodes as absent when it is absent, incomplete
or malformed: the decoder tries the child and drops it on any error, while the
parent still checks its own fields.

`TestBrowserSchemas`, `TestBrowserScalarVariants`, `TestBrowserOptionalFlatten`,
`TestBrowserNegativeBoundsAndNullable` and `TestManifestNullableShapes` read
these files. `verify.mjs` runs the generated Zod against the browser examples
and the mixed scalar union; `TestBrowserTypeScript` runs it with the
`acceptance` build tag after `npm ci` in the parent `testdata` directory. No
server, browser, model or network is used.
