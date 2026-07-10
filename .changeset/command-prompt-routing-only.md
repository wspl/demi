---
"@demicodes/shell": patch
"@demicodes/coding-agent": patch
---

`prompt` is the help pseudo-subcommand only at nodes that route to
subcommands. At a pure run node it is an ordinary argument again, so a
positional literally named "prompt" (e.g. `demi read prompt` for a file
named `prompt`) executes the command instead of printing help. Leaf docs
remain fully reachable through the parent/root help render.
