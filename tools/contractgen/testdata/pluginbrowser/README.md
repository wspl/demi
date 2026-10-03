# Browser plugin schema modes

`manifest.json` is an unchanged copy of g-browser's `TestManifestMatchesFixture` fixture
(`internal/plugins/browser/testdata/manifest.json`), originally captured from
the Rust plugin. No Go-produced expected schema is stored here.

`types.go` copies g-browser's page contract declarations and imports the actual
browserop contracts. `TestBrowserPluginSchemas` compares all eight non-null
page and stream schemas as compact bytes, including definition discovery
order and nullable references. The three null method results are not generated
Go contract types; their declarations remain the page owner's responsibility.

The test adds g-browser's two live-message schema markers through an in-memory
loader overlay because those markers are not present on this base branch. It
checks the generated literals without writing either browserop or g-browser.
The production owner calls `*PluginJSONSchema()` for page/stream uses and keeps
`*JSONSchema()` for commands. Both forms are generated from the same type.
