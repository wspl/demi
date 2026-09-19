---
"@demicodes/command-protocol": minor
---

A native service admits every invocation: `MAX_INVOCATIONS` is gone, each invocation is paced by its own stream window, and cancelling many just-sent invocations no longer closes the service's connection.
