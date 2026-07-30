---
"@demicodes/shell": minor
---

Remove `Command.examples` from the command contract and help renderer. Help stays declarative (summary, usage, parameters, outputs) so models compose from the contract instead of overfitting to canned invocation strings. See `docs/command-help.md`.
