---
"@demicodes/shell": minor
"@demicodes/agent": patch
"@demicodes/coding-agent": patch
---

Replace the `prompt` pseudo-subcommand with a standard `--help` flag.
`--help` renders a node's documentation at every level — groups, dual-mode
parents, leaves, and bare run-only roots — and wins wherever it appears among
a command's arguments. Because help is a flag, it can never collide with
subcommand names or positional values: the reserved-`prompt` child validation
and the routing-precedence rule are gone, and `prompt` is an ordinary name
again. Help-rendering APIs follow the concept: `renderCommandHelp`,
`CommandRegistry.renderHelp()`, and `COMMAND_HELP_DEFAULTS` (which now
advertises `--help`) replace the `*Prompt` names.
