---
'@demicodes/shell': minor
'@demicodes/host-local': minor
'@demicodes/agent': minor
---

Command artifacts are now plain files on the host filesystem; the `/@` virtual filesystem is gone.

`Host` gains `commandArtifactsDir` — a directory reachable through `Host.fs` and visible to processes from `Host.process.spawn` (one shared filesystem namespace). Each command writes `meta.json` / `stdout.txt` / `stderr.txt` (and full `stdout.bin` for binary streams, no longer capped or base64-wrapped) under `<dir>/<storageId>/<commandId>/`, and every status/view path (`artifactDir`, `stdout.path`, placeholders) names those real files. Any tool — a portable in-shell command, a real spawned process, or the embedder's own tooling — reads and searches artifacts with ordinary file operations; embedders on virtual hosts point `commandArtifactsDir` at their virtual disk and portable commands keep working through the same paths.
