# Fork Implementation Comparison

Status: source review and design recommendations, not a runtime implementation.

## Reviewed revisions

Both official repositories were shallow-cloned from their default branches on
2026-09-11 (Asia/Singapore). Findings apply to these exact revisions:

| Project | Branch | Commit |
|---|---|---|
| OpenCode | `dev` | `b7ca4f91d9222dddb8487e689d3274a732999fd9` |
| Codex | `main` | `818f1cca8ccf8899f0f4d59336baebaccf358eed` |

Codex coverage is the public Rust core and app-server. This review does not
establish behavior of the separately distributed Codex desktop interface.
Implementation and existing test assertions were read; the upstream test suites
were not executed. No real models were called.

## Comparison

| Concern | OpenCode | Codex public app-server/core |
|---|---|---|
| Boundary | Copy messages before `messageID`; omit the selected message. | `lastTurnId` includes a terminal turn; `beforeTurnId` excludes a selected turn. Omitting both selects the latest persisted history. |
| UI entry | The app's Fork dialog selects a user message and restores its content to the new composer. | Desktop message actions are outside the reviewed source. |
| Persistence | Create a session and write new message/part records. | Copied history for the legacy path; bounded source-history references for paginated history. |
| Source activity | Fork handler does not require idle or cancel the source. | A terminal prefix can be forked while later work runs. A latest mid-turn snapshot gets an interruption boundary in the destination. |
| Todo/checklist | Current todo rows are separate; Fork does not copy or reconstruct them. | `update_plan` emits a transient plan event; its ordinary tool call remains in persisted model history. No generic versioned command store is restored by Fork. |
| Subagents | Copy parent transcript references; do not recursively copy child sessions or execution. | Create a fresh root runtime and agent registry; do not recursively snapshot child histories or transfer their execution. |
| Files | Fork uses the instance directory/workspace selection; no filesystem snapshot in this operation. | Fork defaults to the source cwd through configuration loading; no worktree creation in this operation. |
| Title | Append `(fork #1)` or increment an existing numbered suffix. | Inherit an explicit source name; the reviewed app-server does not append `(Fork)`. |

The details below qualify these summaries, especially subagent control and
auxiliary state. Neither implementation is a complete snapshot of every
resource associated with a conversation.

## OpenCode

### Message copying

The [HTTP handler][oc-http] calls [Session.fork][oc-fork]. The session service
creates the destination, reads source messages in chronological order, selects
the prefix, and writes new message and part IDs. It remaps assistant parent
message IDs and compaction tail references. Other part payloads are copied.
Creation and all copied records are not enclosed in one transaction by this
function.

An absent `messageID` copies all messages. A supplied ID that is not found also
copies all messages in this implementation. Demi should retain its proposed
explicit error for an unavailable boundary.

The [app dialog][oc-dialog] lists user messages. Selecting U2 from
`U1 → A1 → U2 → A2` creates `U1 → A1` and places U2 in the destination composer.
This differs from Demi's assistant-footer action, which retains the selected
assistant message. The [session test][oc-tests] explicitly asserts the
exclusive chronological prefix, including mixed message-ID ordering.

### Todo state

[Todo.update][oc-todo] deletes the session's current rows and inserts the
replacement list inside a transaction. It has no historical revision parameter.
[The todowrite tool][oc-todowrite] also returns the complete list as tool output
and metadata, so a copied transcript can contain earlier todo lists. That is
distinct from restoring the destination's current todo table. `Session.fork`
does not do that restoration; the new session has no copied todo rows.

### Children and background completion

[The task tool][oc-task] stores child session IDs in metadata and tool output.
Fork copies those payloads without allocating corresponding child sessions.
The task tool's background completion callback captures the original caller's
`ctx.sessionID`, so the already-running job continues delivering there.
Background mode is gated by an experimental runtime flag at this revision.

There is a separate consequence: supplying an old `task_id` causes the tool to
look up that existing session and reuse it. This path does not check that the
reused session's `parentID` equals the invoking session. Thus the source code
permits reuse of an accessible original task from a fork; a copied task reference
is not a frozen historical child. This is a code-path finding, not a separately
executed cross-fork integration test.

## Codex

### Boundary selection and persistence

[thread_fork_inner][cx-fork] loads source history and configuration, resolves the
boundary, and delegates creation to [ThreadManager][cx-manager]. It does not
interrupt the source. The manager creates a new thread identity and fresh
`AgentControl` rather than cloning the source runtime.

For a named inclusive boundary, [rollout truncation][cx-truncation] rejects an
unknown, noncanonical, or in-progress turn. A terminal turn can include an aborted
turn; this is not a claim that only successful assistant messages are eligible.
The legacy implementation cuts before the next turn start. Paginated history
uses the stored terminal position. Neither is an arbitrary assistant text-block
completion contract like Demi's proposal.

For the latest snapshot ending mid-turn, the manager adds an interruption marker
and `TurnAborted` event to the destination's history. The source continues under
its original lifecycle. [The fork tests][cx-tests] check named boundaries,
interruption markers, and exclusion of source content appended after capture.

[Paginated fork preparation][cx-paginated] flushes/materializes history under
storage lifecycle/writer coordination, then records a `history_base` with an
exclusive ordinal and byte offset. It retains that bounded prefix by reference;
it does not copy the complete prefix into the new rollout file. An upstream test
asserts both the reference and the absence of the source prompt in that file,
then checks that the next model request still receives the inherited prompt.
This storage strategy carries lineage and retention responsibilities beyond
Demi's proposed independent destination database.

### Checklist and other state

[PlanHandler][cx-plan] parses `update_plan`, emits `PlanUpdate`, and returns
`Plan updated`. It does not write a todo database or generic command-state
snapshot. [Rollout policy][cx-policy] marks `PlanUpdate` as transient, while
persisting ordinary function calls and outputs. Consequently, copied model
history contains the plan tool's arguments where those calls are retained.
This does not establish a durable, independently queryable todo state or promise
restoration of a desktop plan panel.

Codex does copy selected auxiliary state deliberately. For example,
`deferGoalContinuation`, when enabled with persistent goals, causes the
app-server to flush source goal progress and [copy its current goal record][cx-goal].
That record is read at Fork time, not versioned at `lastTurnId`. Permissions,
plugin settings and other configuration also have explicit inheritance rules.
It would therefore be inaccurate to describe Codex as copying only messages,
or as restoring every kind of state to one historical point.

### Children and control authority

[AgentControl][cx-control] owns a registry scoped to a root session tree.
`thread/fork` constructs a fresh control instance. Its existing-child completion
watcher retains the original parent ID; Fork does not create another watcher for
each source child. The forked parent transcript can still contain earlier spawn
calls and delivered results, but there is no recursive historical child snapshot
in this creation path.

Resuming a root is a different path: [resume_thread_with_history][cx-resume] restores V2
agent metadata by querying open spawn descendants of that resumed root's own
ID. It does not use the root's Fork ancestry to adopt source children. Thus
reopening a fork does not recover the source's children as its own agents.
Their original sessions remain associated with the source; only content already
present in the retained parent history is inherited by the fork.

Control behavior must distinguish the two multi-agent tool versions present in
this revision:

- V2 [message/follow-up][cx-message] and [interrupt][cx-interrupt] handlers require
  the target to be known in the invoking tree's registry. The new fork does not
  inherit registrations for the source's children.
- V1 [send_input][cx-v1] accepts a thread ID and reaches the manager-backed
  `AgentControl.send_input`. That method looks up the target in the shared
  manager. An already-loaded source child can therefore remain addressable in
  that path; a fresh registry alone is not a universal control boundary.

These are traced implementation paths. The reviewed `thread_fork_multi_agent`
tests exercise protocol-version inheritance, not recursive snapshots or all
cross-fork child-control cases.

## Recommendations for Demi

These recommendations inform the proposed [Fork design](conversation-fork.md)
and [Command Storage History](command-storage-history.md). Product decisions
under discussion remain unconfirmed.

1. Keep history selection in the agent framework and product creation in the
   backend. Both projects support this division; providers do not need a new
   Fork protocol. Define a validated replay boundary for Demi's assistant text
   action instead of assuming a Codex turn ID is equivalent.
2. Allow a running source. Capturing a stable prefix can use short persistence
   coordination without waiting for the whole turn or cancelling source work.
3. Keep command-state history if Fork and editing must restore todos accurately.
   OpenCode leaves a gap here; Codex's plan representation does not solve Demi's
   independently mutable `CommandStorage`. This requirement comes from Demi's
   state contract, not from matching either upstream implementation.
4. Make child execution ownership mandatory and explicit. Source children keep
   running for the source; copied IDs never authorize controlling them from a
   destination. Enforce this at command dispatch, not only in the interface.
5. Treat full frozen child history as a separate product capability. A simpler
   initial Fork can preserve only the child references/results already present
   in the root transcript and label them as historical, with no active controls
   or promise to show the child's complete history at that point. If viewing
   that exact child history is required, the proposed immutable tree snapshots
   remain necessary. Copying the child's current history when Fork is clicked
   cannot satisfy that requirement.
6. Start with independently owned copied history in Demi's existing database
   layout. Codex's bounded references are a possible future optimization after
   lineage retention and deletion semantics are designed.

The largest unresolved scope decision is exact child-history viewing. It should
not become a prerequisite merely because Fork must isolate live execution.
Todo restoration and child execution isolation can be specified independently
of that viewing capability.

[oc-http]: https://github.com/anomalyco/opencode/blob/b7ca4f91d9222dddb8487e689d3274a732999fd9/packages/opencode/src/server/routes/instance/httpapi/handlers/session.ts#L206
[oc-fork]: https://github.com/anomalyco/opencode/blob/b7ca4f91d9222dddb8487e689d3274a732999fd9/packages/opencode/src/session/session.ts#L691
[oc-dialog]: https://github.com/anomalyco/opencode/blob/b7ca4f91d9222dddb8487e689d3274a732999fd9/packages/app/src/components/dialog-fork.tsx#L34
[oc-tests]: https://github.com/anomalyco/opencode/blob/b7ca4f91d9222dddb8487e689d3274a732999fd9/packages/opencode/test/session/session.test.ts#L241
[oc-todo]: https://github.com/anomalyco/opencode/blob/b7ca4f91d9222dddb8487e689d3274a732999fd9/packages/opencode/src/session/todo.ts#L29
[oc-todowrite]: https://github.com/anomalyco/opencode/blob/b7ca4f91d9222dddb8487e689d3274a732999fd9/packages/opencode/src/tool/todo.ts#L13
[oc-task]: https://github.com/anomalyco/opencode/blob/b7ca4f91d9222dddb8487e689d3274a732999fd9/packages/opencode/src/tool/task.ts#L136
[cx-fork]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/app-server/src/request_processors/thread_processor.rs#L4848
[cx-manager]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/thread_manager.rs#L1320
[cx-truncation]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/thread_rollout_truncation.rs#L159
[cx-paginated]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/thread-store/src/local/paginated_fork.rs#L16
[cx-tests]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/app-server/tests/suite/v2/thread_fork.rs#L567
[cx-plan]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/tools/handlers/plan.rs#L68
[cx-policy]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/rollout/src/policy.rs#L40
[cx-goal]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/app-server/src/request_processors/thread_fork_goal.rs#L5
[cx-control]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/agent/control.rs#L118
[cx-message]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/tools/handlers/multi_agents_v2/message_tool.rs#L53
[cx-interrupt]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/tools/handlers/multi_agents_v2/interrupt_agent.rs#L35
[cx-v1]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/tools/handlers/multi_agents/send_input.rs#L33
[cx-resume]: https://github.com/openai/codex/blob/818f1cca8ccf8899f0f4d59336baebaccf358eed/codex-rs/core/src/thread_manager.rs#L1144
