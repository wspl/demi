# Browser plugin schema modes

`manifest.json` is a copy of the browser plugin's manifest fixture
(`internal/plugins/browser/testdata/manifest.json`). Its `BrowserTab` and
`LiveTab` definitions include the required `loading` boolean and its doc
description.

`types.go` copies the browser plugin's page contract declarations and imports
the real `browserop` contracts. `TestBrowserPluginSchemas` compares all eight
non-null page and stream schemas as compact bytes, including definition
discovery order and nullable references. The three null method results are not
generated contract types; their declarations remain the page owner's.

A page or stream use calls `*PluginJSONSchema()`; a command keeps
`*JSONSchema()`. Both forms are generated from the same type.
