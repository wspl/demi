---
"@demicodes/utils": minor
"@demicodes/core": minor
"@demicodes/shell": major
"@demicodes/agent": minor
"@demicodes/coding-agent": minor
"@demicodes/host-local": patch
---

Binary streams end to end, attachment channel removed. Pipes are byte-clean
through real OS processes in both directions (`hostSpawn` stdin/stdout were
UTF-8-lossy); the exec boundary classifies the final stream — valid UTF-8 is
text, anything else surfaces as `binaryStdout` (raw bytes, truncation-aware)
with a placeholder text render. The agent layer sniffs the closed model-media
set by magic bytes and attaches image/video blocks when the model accepts the
type, explaining why otherwise. `CommandAsset` / `io.asset()` and every
`supportedAssetTypes` thread are gone; `demi read` emits raw file bytes
(media presentation happens at the boundary); the command bridge carries
binary stdout as base64 and the shim writes raw bytes to its OS stdout.
