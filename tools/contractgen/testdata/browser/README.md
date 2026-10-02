# Browser contract fixtures

`types.go` mirrors the shapes involved in f-contractgen7 from
`crates/command-package-browser-protocol/src/browser/{mod,operations,failure}.rs`.
`Wheel` carries the two negative ranges from `src/live.rs`'s wheel variant.
`Scalar` additionally exercises overlapping numeric variants, boolean and object
variants. `Optional` checks embedding between sibling properties.

The schema snapshots are extracted without changing property schemas from
`gomig-ref/plugin-manifests/manifests.json`:

- `NodeValue`: browser `inspect` result, `tree.items.properties.value`.
- `ErrorDetails` and `BrowserFailure`: browser `content.fetch` result,
  `pages.items.properties.error` and its `details` property.
- `DialogInspectResult`: browser `dialog.inspect` result, whole schema.

Nested snapshots add only a root `title`, as generated schema roots do.
`nullable-manifest.json` preserves every nullable schema occurrence in that
manifest, keyed by its JSON pointer. They cover plain strings, date-time and
bounded strings, objects, and references.

`failure.json` is the export-details document asserted by Rust's
`results_and_failures_print_the_documented_names`, using its tab `t1`.
The rest of that Rust test, and the other browser protocol tests, belong to
c-browserop; this generator fixture does not duplicate that protocol package.

Flattened optional decoding follows serde 1.0.229's `FlatMapDeserializer` and
`OptionVisitor::__private_visit_untagged_option`: it tries the child and uses
`.ok()`. Thus an absent, incomplete, or malformed export becomes `None`.
The parent still checks its own fields. Schemars 1.2.2's `allow_null` determines
the nullable-schema representation. These source-defined cases are regression
scenarios, not a claim that Rust was executed to produce a new byte corpus.

`verify.mjs` executes the generated Zod against the browser examples and the
mixed scalar union. Run with the Go `acceptance` tag after `npm ci` in the parent
`testdata` directory. No server, browser, model, or network is used by tests.
