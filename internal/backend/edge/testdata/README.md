`routes.txt` lists every method and path the backend serves, one per line and sorted, with the `/api` prefix written out and the native artifact route at `/native-artifacts`. `TestRoutesMatchReferenceList` fails when a route is omitted, added or changed.

The d-panel routes are transcribed from `crates/backend-http/src/lib.rs` at `efc8fbe6e`.
