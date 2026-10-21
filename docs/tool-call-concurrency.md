# Tool-call concurrency

## Agent execution

`@demicodes/agent` splits the tool calls of one provider response into batches
in response order. Adjacent calls of tools that declare `concurrent: true` form
one batch and start together; every other call is a batch of its own, so it runs
after every earlier call has finished and before any later one starts. The
standard shell tools (`shell_exec`, `shell_status`, `shell_write`,
`shell_abort`) are concurrent; `yield` and host tools without the flag are not.

Each result is recorded as soon as it arrives, one at a time, so transcript
commits never overlap, and the next provider request carries the results in the
response's order. Steers that arrive during a batch are materialized after it,
never between calls that started together. An abort waits for every call of the
batch to settle before the turn ends.

`BashEnvironment.exec` picks a shell and starts the script on it as one step, so
concurrent `shell_exec` calls without `shellId` each run on an idle shell instead
of both taking the session's default shell.

## Claude Code SDK-MCP

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
| `packages/agent/src/__tests__/concurrent-tools.test.ts` | Adjacent concurrent calls start before either finishes; a sequential call runs alone between batches; results reach the next request in response order. |
| `packages/shell/src/__tests__/concurrent-exec.test.ts` | Concurrent execs of one agent session run on separate shells, with and without an idle default shell. |
| `packages/provider-claude-code/src/__tests__/provider.test.ts` | Two model-emitted tool uses are yielded in one run before any SDK-MCP response; a deferred second callback consumes its already-buffered result. |
| `packages/provider-claude-code/src/__tests__/real-cli.e2e.test.ts` | Opt-in smoke test against the installed Claude CLI verifies a real two-tool SDK-MCP batch and continuation. |
