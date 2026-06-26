# @demi/provider-claude-code

A Demi provider backed by the Claude Code CLI. Exposes `createClaudeCodeProvider()`
and `listClaudeCodeModels()`.

```ts
import { createClaudeCodeProvider, listClaudeCodeModels } from '@demi/provider-claude-code'
```

> Diagnostics: the transport writes a raw request/response wire log (including
> prompts) to `$TMPDIR/demi-claude-wire` by default. Disable with
> `DEMI_CLAUDE_WIRE_LOG=0`. See [SECURITY](../../SECURITY.md).

Implements the [`@demi/provider`](../provider/README.md) contract. Part of
[Demi](../../README.md). Apache-2.0.
