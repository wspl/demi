# Tool Rendering Specification

| | |
|---|---|
| Date | 2026-06-25 |
| Status | Final specification |
| Scope | How the standard agent tools render across the Web UI, the REPL, and other shells |

## 1. Goals

Demi's underlying transcript and event protocol already express tool calls fully:
`@demicodes/core` defines `Block`, and `@demicodes/agent` defines `ClientSessionEvent`. Tool
rendering neither needs nor should introduce a new model / render-model package.

The principles for rendering the standard tools:

1. `tool_call` is the persistent transcript envelope, not a render type.
2. A shell dispatches first on `block.type`; once it sees `type === "tool_call"` it
   must then dispatch on the concrete `block.toolName`.
3. The Demi standard tools — `shell_exec` / `shell_status` / `shell_write` /
   `shell_abort` / `yield` — must each have a first-class rendering; they must never
   fall through to the generic tool card.
4. The generic tool rendering is only for unknown external tools or future extension
   tools, never for the standard tools.
5. The Web UI and the REPL may each implement their own DOM / terminal UI, but they
   must consume the same `Block` and `ClientSessionEvent` data — never copy the
   protocol or introduce a parallel data model.

## 2. Shared protocol boundary

Persistent history is governed by `Block`:

- `Block.type === "tool_call"` means the model issued a tool call.
- `toolName` is the dispatch key for standard-tool rendering.
- `input` is the JSON string the provider supplied; the render layer parses it.
- `status` is `executing | completed | error`.
- `streamingOutput` / `output` is the tool's output text or media blocks.
- `metadata` may carry structured runtime state such as a `ShellCommandSnapshot`; the
  render layer may use it to enrich the display, but must not treat it as the sole source.

Real-time events are governed by `ClientSessionEvent`:

- `transcript_snapshot` / `transcript_patch` are the primary input for the persistent UI.
- `shell_output` / `tool_progress` may add live stdout/stderr or status to a standard
  tool that is currently executing.
- `shell_write_result` / `abort_result` are acknowledgements of user control actions;
  they do not replace the `tool_call` rendering in the transcript.
- `audit` may surface registered-command or system-command detail inside the
  `shell_exec` card, but must not change the standard-tool dispatch key.

So the Web UI, the REPL, and future shells share the protocol and event structures, but
not a single abstract UI-model package.

## 3. The `description` convention

Every standard tool's input schema must allow an optional `description?: string`.

`description` is a short, user-visible intent title. It should let the user understand
which concrete user-visible state or result this step is meant to surface, confirm, or
advance — not the tool mechanics.

Rendering rules:

1. A non-empty `description` is the preferred title for the tool block.
2. With no `description`, the render layer uses each tool's deterministic fallback.
3. `description` affects display only; it changes neither the shell runtime, the tool
   result, nor model-replay semantics.
4. `description` should not describe waiting, pausing, or tool mechanics; should not be a
   generic action name or a bare noun; and should not be stuffed with long scripts, full
   stdout/stderr, protocol state, step numbers, the toolName, the commandId, internal
   labels, or rationale.

## 4. Standard-tool rendering

| Tool | Render form | Title fallback | Key content | In-progress state |
|---|---|---|---|---|
| `shell_exec` | Terminal command block | `input.script` | The script, plus stdout/stderr terminal output interleaved in arrival order | Sweep loading; output is expandable |
| `shell_status` | Command-status inline block | `Check <commandId>` | status, runningMs, idleMs, bytes, artifact paths; no output body; no expand panel | Sweep loading; must not masquerade as shell_exec |
| `shell_write` | stdin-write inline block | `Send input to <commandId>` | Preferred title states the user-visible result being advanced; no expand panel | Sweep loading; success ≠ command completion |
| `shell_abort` | Stop-command inline block | `Stop <commandId>` | Preferred title states the user-visible state being settled; no expand panel | Sweep loading; completed/aborted are not UI errors |
| `yield` | Wait-for-wakeup inline block | `Wait <durationMs>ms` | Preferred title states the user-visible state to observe or confirm next; no expand panel | Sweep loading consistent with thinking |

These tools may share one base `ToolCard` shell, but the content area, title fallback, and
status copy must be differentiated by tool name. `shell_exec` / `shell_status` /
`shell_write` / `shell_abort` all use the terminal icon; `yield` uses a clock/timer. The
in-progress state for every standard tool uses the same sweep loading as thinking — never
a separate spinner.

In the Web UI, only the `shell_exec` tool block and the `thinking` block are expandable.
`shell_status` / `shell_write` / `shell_abort` / `yield` and unknown generic tools must
stay as non-expandable inline renderings; if an error needs to be shown, it appears only as
a badge or an inline summary, never behind a disclosure.

The `shell_exec` expanded content shows only the command and the user-visible terminal
output. That terminal output comes from the interleaved output stream in runtime/progress
or from the auto-budgeted preview, merged into a single transcript in stdout/stderr arrival
order; this is purely a UI/runtime rendering and is not saved as a `/@` file. The
authoritative full-output read path the model sees is
`/@/commands/<commandId>/stdout.txt` and `stderr.txt` — not `shell_status`. When an older
transcript lacks interleaved output it may fall back to stdout-then-stderr, but it must not
show stderr alone, nor show protocol fields such as `status`, `shellId`, `commandId`, path,
offset, bytes, or truncation.

`shell_status` may show only a command-status summary. Even when its metadata carries
artifact paths, byte counters, or a preview, it must not be rendered as an expandable
terminal-output block; the user path to output content is to view the corresponding
`shell_exec` block or to have the model read the `/@` artifact with a shell text command.

## 5. Web specification

The Web UI's `ToolCallBlock` must dispatch explicitly:

```text
shell_exec    -> shell exec renderer
shell_status  -> shell status renderer
shell_write   -> shell write renderer
shell_abort   -> shell abort renderer
yield         -> yield renderer
unknown       -> generic renderer
```

`AgentMessageVirtualBlock` still does the first-level dispatch on `block.type`;
`ToolCallBlock` carries the second-level `toolName` dispatch. The virtual list, sticky user
block, auto-scroll, tail loading, and so on remain Web-private UI implementation details
and do not enter the shared protocol layer.

The Web UI must avoid two mistakes:

- Do not render the standard tools as a generic card just because `type === "tool_call"`.
- Do not disguise `shell_status` / `shell_write` / `shell_abort` / `yield` as `shell_exec`
  command output; they are distinct control actions.

## 6. REPL specification

The REPL consumes the same `Block` and `ClientSessionEvent`, but its output is terminal lines.

Minimum requirements:

- `shell_exec` prints `tool> shell_exec ...` and shows the script fallback.
- `shell_status` prints the commandId and a status-result summary; no stdout/stderr body.
- `shell_write` prints the commandId and a summary of the stdin's effect; stdin content may
  be truncated.
- `shell_abort` prints the commandId and a stop-result summary.
- `yield` prints the durationMs and a wakeup-result summary.
- When `description` is present it takes precedence for the summary; otherwise use the
  §4 fallback.

The REPL need not reuse Web components and should not introduce a DOM-oriented render model.
It only needs to follow the same tool-name → rendering-semantics mapping as the Web UI.

## 7. Acceptance & testing

Must cover:

1. Standard-tool schema: all five tools allow `description`, and the standard-tool set is
   still exactly `shell_exec/shell_status/shell_write/shell_abort/yield`.
2. Web dispatch: `ToolCallBlock` has a dedicated renderer for each of the five standard
   tools; only unknown tools fall through to generic.
3. Web rendering: all five standard tools prefer `description`, falling back to §4 when absent.
4. REPL rendering: the terminal summaries for all five standard tools prefer `description`,
   fall back to §4 when absent, and avoid duplicate output on patch replay.
5. Protocol stability: when `transcript_patch` updates the status/output/metadata of the
   same `tool_call`, both the Web UI and the REPL update the same render block rather than
   appending an erroneous duplicate.

Real-model acceptance must cover at least one long-command flow:

```text
shell_exec -> yield -> shell_status -> shell_write or shell_abort
```

Acceptance should confirm that both the Web UI and the REPL make clear what each control
action is doing, rather than showing one undifferentiated group of tool calls.
