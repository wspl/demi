# Command help

Registered `Command` trees render into the agent's `commandsPrompt` and
`--help` through `renderCommandHelp` in `@demicodes/shell`. The parser and
renderer use the same input declarations. Each field has one input source:

- `stdinField`: a text body supplied through a quoted heredoc, pipe, or input
  redirection. It has no option or positional form.
- `positionals`: ordered arguments. They have no named-option aliases.
- `restField`: raw arguments after `--`. It has no named-option alias.
- Other input fields: named options. Their schemas define values, optionality,
  boolean flags, enums, and repeated array options.

Registration rejects overlapping input sources, duplicate positional fields,
required positionals after optional ones, and options named `help` or `json`.
The parser rejects repeated scalar values and reports missing option values
before consuming the next option. `--name=value` supplies an option value
beginning with `--`; a standalone `--` ends option parsing for positionals or
starts the declared raw-argument field.

Help displays a complete usage template, value placeholders, required and
optional arguments, enum choices, and repeatable options. Stdin bodies appear
in their own section and in a quoted heredoc template. `--json` appears only
on commands with a JSON output schema. Usage templates are generated from the
contract; command specs do not carry separate examples to keep in sync.

For example, the generated structure for file creation is:

```sh
demi file create <path> <<'EOF'
<content>
EOF
```

Angle brackets identify placeholders; square brackets identify optional
arguments. The agent substitutes actual values and quotes shell arguments.
A body option such as `--content` is rejected with a diagnostic directing the
caller to remove it and use stdin.

## Demi command inputs

| Command | Positional input | Named options | Stdin |
| --- | --- | --- | --- |
| `file read` | path | none | unused |
| `file create` | path | none | file content |
| `file edit` | path | old, new, occurrence, context | unused |
| `file patch` | none | none | unified diff |
| `todo add` | text | JSON output | unused |
| `todo update` | id | text, status, JSON output | unused |
| `todo done` | id | JSON output | unused |
| `todo list` | none | JSON output | unused |
| `agent spawn` | none | profile, description, no-subagents, JSON output | task brief |
| `agent send`, `steer`, `resume` | id | JSON output | message |
| `agent abort`, `show` | id | JSON output | unused |
| `agent list` | none | JSON output | unused |
| `host shell` | one quoted script | host | streamed to the remote program |
| `host list`, `current` | none | none | unused |

`file edit` has two separate text operands; its old and new values are quoted
option arguments. Use `file patch` for a multiline change supplied as one
heredoc. `host shell` takes the script as one quoted argument so its stdin
remains available for the remote program's data. These commands do not claim
that stdin supplies their script or edit operands.

## Test coverage

The shell tests cover exclusive input sources, diagnostics, complete usage,
option value handling, and schema-derived help. Command-loader tests verify
the same contract after manifest serialization. Coding-agent and subagent
tests execute heredoc bodies through the runner without calling real models.
