# Tool-call concurrency

Demi treats all tool calls emitted by one provider response as one execution
batch. `@demicodes/agent` starts every invocation in that batch concurrently,
waits for the batch to settle, then records results and lifecycle events in the
model's original order. A later provider round receives the complete result
batch.

This boundary is deliberate:

- Providers define the batch by ending one `AgentProvider.run()` after emitting
  the response's tool-call events.
- The Agent owns execution concurrency. Providers do not start tools.
- Transcript writes remain ordered even though the tool work is concurrent.
- Steers arriving during execution are materialized after the current batch,
  never between calls that already started together.

## Claude Code SDK-MCP

Claude Code can emit several `tool_use` blocks in one assistant message while
its SDK-MCP control channel exposes only the first `tools/call`. The next
callback may remain blocked until the first result is returned.

`@demicodes/provider-claude-code` therefore collects the assistant's original
`tool_use` blocks through `message_stop` and yields the complete batch with the
original `claudecode/toolUseId` values. When the Agent returns all results, the
provider answers callbacks already available, continues reading the live CLI
stream, and immediately matches each later callback to its buffered result.
This keeps one Claude process and one SDK-MCP session; it does not add a proxy
process or alternate MCP transport.

## Test coverage

| Module | Intended coverage |
| --- | --- |
| `packages/agent/src/__tests__/session.test.ts` | Both invocations in one provider tool batch start before either is released; transcript results retain provider order. |
| `packages/provider-claude-code/src/__tests__/provider.test.ts` | Two model-emitted tool uses are yielded in one run before any SDK-MCP response; a deferred second callback consumes its already-buffered result. |
| `packages/provider-claude-code/src/__tests__/real-cli.e2e.test.ts` | Opt-in smoke test against the installed Claude CLI verifies a real two-tool SDK-MCP batch and continuation. |
