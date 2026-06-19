# Demi TUI

A small acceptance shell for exercising the coding agent through the same
AgentServer, provider, and shell layers used by embedders.

## Run

Use a scratch workspace when testing file edits:

```sh
mkdir -p /tmp/demi-acceptance
bun run tui -- /tmp/demi-acceptance
```

The TUI uses the Claude Code provider, so sending a prompt may call the real
model. By default it passes a small max budget:

```sh
bun run tui -- --cwd /tmp/demi-acceptance --budget 0.25
```

Useful options:

```sh
bun run tui -- --help
bun run tui -- --model claude-opus-4-8
bun run tui -- --yield-after-ms 250
bun run tui -- --timeout-ms 120000
```

Omit `--model` to use the provider catalog default.

## What To Verify

Try a prompt that forces planning, file edits, shell execution, and follow-up
progress:

```text
Create src/app.ts with an exported add(a, b) function, add a README with a short usage example, run cat on both files, and tell me what changed.
```

The terminal should show:

- `thinking>` streamed reasoning blocks when the provider exposes them.
- `assistant>` streamed assistant text deltas.
- `tool:` lifecycle changes for shell tools.
- `progress:` shell session progress, including yield/running status.
- `shell[...] stdout>` and `shell[...] stderr>` live shell output.
- `status:` phase changes for queued/running/idle state.

## Runtime Commands

```text
/help
/abort
/retry
/resume
/compact
/input <shellId> <text>
/exit
```

Messages are sent asynchronously, so `/abort` can be entered while a turn is
still running.
