# Command help

Registered `Command` trees render into agent `commandsPrompt` and `--help` via
`renderCommandHelp` in `@demicodes/shell`. Help carries summary, usage, outputs,
parameters, stdin/heredoc fields, `--json`, and subcommands.

Command specs intentionally omit invocation examples. Concrete example lines
tend to overfit the model toward copying those strings instead of composing from
the parameter contract, so help stays declarative.

## Test coverage

| Module | Intended coverage |
| --- | --- |
| `packages/shell/src/__tests__/command.test.ts` | Help rendering and registry validation for the declarative command contract (no example fields). |
