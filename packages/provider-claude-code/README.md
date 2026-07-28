# @demicodes/provider-claude-code

A Demi provider backed by the Claude Code CLI. Exposes `createClaudeCodeProvider()`
and `listClaudeCodeModels()`.

```ts
import { createClaudeCodeProvider, listClaudeCodeModels } from '@demicodes/provider-claude-code'

const provider = createClaudeCodeProvider()
// provider.auth, provider.quota, provider.credentials (multi-account pool by default)
```

## Auth and credentials

- Default material: `CLAUDE_CODE_OAUTH_TOKEN` or macOS Keychain (`Claude Code-credentials`).
- Multi-credential pool under `$DEMI_HOME/credentials/claude-code/`; active token is
  injected into the CLI child as `CLAUDE_CODE_OAUTH_TOKEN`.
- Lifecycle: `beginLogin` → `claude auth login` → `importDefault` / `add` → `setActive`.
- Changing active credential forces a cold restart of a long-lived CLI process.

See [docs/provider-global-credentials.md](../../docs/provider-global-credentials.md).

## Quota

- **probe** (cost: `free`): `GET /api/oauth/usage` with the active OAuth token.
- **observe**: stream-json `rate_limits` bodies (and unified rate-limit headers when present).

See [docs/provider-quota.md](../../docs/provider-quota.md).

## Tool-call batches

Claude Code's SDK-MCP control channel can hold later `tools/call` callbacks until
the preceding callback receives a result, even when the model emitted several
`tool_use` blocks in one response. The provider preserves the model's original
batch and tool-use IDs for the host scheduler, then matches the completed
results to SDK-MCP callbacks as the CLI releases them.

See [docs/tool-call-concurrency.md](../../docs/tool-call-concurrency.md).

> Diagnostics: the transport writes a raw request/response wire log (including
> prompts) to `$TMPDIR/demi-claude-wire` by default. Disable with
> `DEMI_CLAUDE_WIRE_LOG=0`. See [SECURITY](../../SECURITY.md).

Implements the [`@demicodes/provider`](../provider/README.md) contract. Part of
[Demi](../../README.md). Apache-2.0.
