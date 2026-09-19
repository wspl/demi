---
"@demicodes/runner-protocol": minor
"@demicodes/backend": minor
"@demicodes/web": patch
---

The runner no longer refuses work because other work is in flight. Commands the backend implements and local command connections are not counted; filesystem, working-tree and sync requests wait for a slot instead of answering `busy`, so the working-tree error code `busy` and the `changes_busy` answer are gone. Out of open files, whatever the runner needs one for waits until one closes.
