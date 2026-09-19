---
"@demicodes/runner-protocol": minor
"@demicodes/host-remote": minor
"@demicodes/backend": minor
---

A page can open a user stream: `WS /api/conversations/:id/streams/:name` opens a declared stream on the conversation's main Host, checked by session, ownership and `Origin`, admitted without waking a stopped Cloud and ended by an archive, a target or directory change, or a detach. The runner's new `service_open` invokes the stream's operation in the resident native service with a `user` command context and the conversation's directory, and carries the page's bytes in and the invocation's standard output out through two pipes, as fast as the page takes them; `service_opened` and `service_error` answer it. `artifact_resolve` now names its owner, a job or a stream, so a stream can start its service. `POST /api/conversations/:id/activity` records one user operation. Backends declare their streams with `userStreams`; the live browser view's `browser` stream is declared when the package provides `browser.live`.
