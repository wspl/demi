# Transcript replay byte fixtures

These independently specified expected bytes pin a replayed session containing
native image bytes, signed reasoning, ordered nested tool arguments, and a tool
result containing text and an image. Both include `<`, `>`, `&`, U+2028 and U+2029.
They are not snapshots generated from the Go implementation.

`request.json` is the complete `provider.InferenceRequest` at transcript's public
boundary, serialized by `provider.JSONBody`. It is not a vendor HTTP body.
`responses-input.json` is that request's complete Responses input through the
shared production `provider.ResponsesInput` renderer. Provider-specific full HTTP
body tests belong to those providers; this package cannot import a vendor.
