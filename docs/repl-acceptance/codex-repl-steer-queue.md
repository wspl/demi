# Codex REPL Steer / Queue Acceptance

Date: 2026-06-23

Status: passed

Scope: real Codex provider, real REPL process, `AgentClient.steer`, queued input, shell-tool active-turn gate.

## Command

A one-off `bun --eval` harness spawned the REPL process with:

```sh
bun run packages/repl/src/index.ts \
  --provider codex \
  --model gpt-5.4 \
  --thinking medium \
  --transport sse \
  --cwd "$TMPDIR/demi-repl-codex-steer-*" \
  --yield-after-ms 250 \
  --timeout-ms 180000
```

Result:

```text
status: passed
model: gpt-5.4
transport: sse
```

## Scenario

1. Start the REPL with the real Codex provider and local Codex auth.
2. Send an active prompt that asks the model to run:

   ```sh
   sleep 12; echo DEMI_REPL_STEER_GATE_RELEASED
   ```

3. Wait until REPL renders the foreground shell as running.
4. Send a normal non-command message while the active turn is running. This must become queued input.
5. Send:

   ```text
   /steer Same-turn guidance: include STEER_REPL_INCLUDED in the active final answer.
   ```

6. Wait for the shell gate to finish, then wait for the active answer and queued answer.
7. Exit the REPL cleanly.

## Observed Output

```text
provider  codex
model     gpt-5.4
transport sse
state> session opened; type /help for commands, /exit to quit
tool> shell_exec executing -- sleep 12; echo DEMI_REPL_STEER_GATE_RELEASED
progress> shell[4ab6f466-612d-4b99-9ce9-5b8cfa5edeea] running (yield)
queue> 1 pending
shell[4ab6f466-612d-4b99-9ce9-5b8cfa5edeea] stdout> DEMI_REPL_STEER_GATE_RELEASED
steer> Same-turn guidance: include STEER_REPL_INCLUDED in the active final answer.
assistant> ACTIVE_DONE STEER_REPL_INCLUDED
assistant> QUEUED_DONE
state> closed
```

The harness asserted that:

- `steer>` rendered before the active final answer.
- the active final answer contained `ACTIVE_DONE` and `STEER_REPL_INCLUDED`;
- the queued answer contained `QUEUED_DONE`;
- `ACTIVE_DONE` appeared before `QUEUED_DONE`;
- the REPL exited with code `0` and no stderr.

## Notes

The accepted REPL scenario uses a timed foreground shell command as the active-turn gate. Earlier exploratory stdin-gate attempts were discarded: one nested Python prompt was not preserved by the model, and a bare shell `read` command exited on EOF instead of keeping the foreground command running. Those failed attempts were not counted as acceptance.
