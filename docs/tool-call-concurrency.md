# Claude Code tool-call batching

Claude Code can emit several `tool_use` blocks in one assistant message, but its
SDK-MCP control channel can expose only the first `tools/call`. A later callback
may remain blocked until the preceding result is returned.

`@demicodes/provider-claude-code` therefore collects the assistant's original
`tool_use` blocks through `message_stop` and yields the complete batch with the
original `claudecode/toolUseId` values. This lets the host see the model's real
batch instead of mistaking delayed SDK callbacks for separate model turns.

When the host returns the batch results, the provider answers callbacks already
available, continues reading the live CLI stream, and immediately matches each
later callback to its buffered result. This keeps one Claude process and one
SDK-MCP session; it does not add a proxy process or alternate MCP transport.

## Test coverage

| Module | Intended coverage |
| --- | --- |
| `packages/provider-claude-code/src/__tests__/provider.test.ts` | Two model-emitted tool uses are yielded in one run before any SDK-MCP response; a deferred second callback consumes its already-buffered result. |
| `packages/provider-claude-code/src/__tests__/real-cli.e2e.test.ts` | Opt-in smoke test against the installed Claude CLI verifies a real two-tool SDK-MCP batch and continuation. |
