# Agent messages

Status: implemented. Acceptance uses scripted providers, isolated stores, and the shared product/gallery UI.

This document defines agent-to-agent delivery, automatic completion receipts,
and their transcript presentation. `subagent.md` owns the rest of the child
lifecycle. Human input and human steering keep their existing contracts.

## Delivery behavior and transcript type

Steering is the delivery behavior: inject input into an active turn through the
existing provider steer or safe-continuation path. An `agent_message` block is
the transcript representation: record input from another agent with its source
and event details. These are independent dimensions.

`AgentSession` reuses its internal steering and hidden-wakeup paths for agent
messages. It extends their input records with the structured agent source and
materializes them as `agent_message` blocks. Human steering continues to
materialize as `steer` blocks. No parallel inbox, scheduler, or provider delivery
loop is introduced for agent messages.

For example, a child finishing while its parent executes a tool supplies an
internal steer. After the tool completes, the existing continuation path
incorporates that receipt and appends one `agent_message` block. The parent
continues the same turn; the UI renders a Bot receipt row from that block.

## Message identity

An agent message is structured input from another agent, not a human message
or the receiving agent's assistant output. The supervisor supplies the sender
identity from the invoking node; models cannot impersonate another sender by
writing an identifier into the message body.

Each message has a stable id, sender node id, sender description, sender round
identifier (`spawnedAt` from the persisted sender node), recipient node id, timestamp, content, and one event variant:

- `message`: an explicit communication between live agents.
- `completion`: a supervisor receipt with outcome `completed`, `failed`, or
  `aborted`, plus the bounded result or failure detail.

The completion id identifies exactly one child execution round. Reopening a
child creates a different round. Source metadata is independent of presentation
text; neither replay nor the UI parses a bracketed text prefix to find it.
Shared data types belong to `core`; `agent` validates command inputs and internal
message envelopes at admission, using explicit schemas.

## One communication operation

The model-facing operation is `demi agent send <id|parent>`. Its meaning is
"deliver this information to that agent." The runtime selects when to deliver;
the sending model does not choose between a mailbox and a steer. There is no
separate agent-to-agent `steer` verb. Human steering remains a distinct action.

`send` resolves after durable acceptance, not after inference or an answer.
It never waits for another agent to finish, so reciprocal messages cannot
block each other's command execution. Archived recipients fail explicitly;
only their parent can reopen them through `resume`.

Automatic completion receipts use the same admission operation. The child
returns its final answer once; the supervisor delivers it. Child instructions
request explicit messages only for useful interim information, questions, or
blockers, not a duplicate of the final completion result.

## Delivery and scheduling

`AgentSession` admits explicit agent messages and completion receipts through
one internal-input entry point. While running, it uses the existing steering
path, including pending steers and their materialization at safe boundaries.
While idle, it uses the existing hidden-wakeup action path. The input record
carries the agent-message variant through either path; internal inputs are
excluded from the human queue and pending-steer UI snapshots.

`pendingInternalSteers` in the target checkpoint serializes the agent-originated
members of `PendingSteerQueue`: the target turn, model, source envelope, and action
metadata. The message id and body live only in the envelope. Admission validates
the envelope and flushes the checkpoint before returning. Materialization replaces
these pending records with transcript blocks in the same target checkpoint.
This extends the existing steering contract without a separately managed inbox.

| Recipient state | Delivery |
|---|---|
| Running with a supported live provider steer | Deliver through that provider's steer capability. |
| Sampling or executing tools without live steering | Consume messages at the next safe continuation boundary, before requesting more model output. |
| Compacting | Preserve messages outside the compacted prefix and consume them before post-compaction inference. |
| Finalizing | Preserve accepted messages; recheck pending internal input before committing idle or closing the child. |
| Naturally idle | Consume the available batch and open one internal continuation, without a human user bubble. |
| User-aborted | Retain unread messages without automatically resuming the aborted work; explicit user continuation can consume them. |
| Archived | Reject explicit sends; a completion receipt remains durable at its owning supervisor until delivery is possible. |

Messages do not cancel an executing tool or restart the current turn. A safe
boundary can be later than the arrival time; the UI must not claim that the
model has read a receipt merely because the runtime accepted it.

At a boundary the existing steer materialization consumes pending messages in
admission order before the next inference request. Distinct messages retain
distinct ids and content. Pending internal wakeups use the existing action
worker and incorporate available agent inputs together, rather than creating
one human send turn per message. Messages arriving after that boundary belong
to the following opportunity.

The session serializes admission with finalization so a finishing/idle race
cannot silently lose a message. Accepted pending internal input participates in
the existing quiescence check and prevents premature child closure.

The receiving agent treats receipts as context for its active task. It does
not owe a separate user-facing acknowledgement for every message. If nothing
else can advance until a child returns, it ends its turn and relies on event
delivery; it does not poll or schedule short timed wakeups.

## Durable ownership and replay

Existing pending steers and hidden-wakeup actions hold accepted inputs until
the session incorporates them into context. The corresponding target checkpoint
materializes an `agent_message` block and consumes the pending input atomically.
The source fields survive both live provider steering and subsequent-request
steering. There is no second mutable "delivered" flag on the block and no
additional message-store lifecycle.

A completion is acknowledged by its source round only when the target has
durably accepted responsibility, in the same transaction as saving the pending
internal input or its materialized transcript block. Restore retries
unacknowledged rounds with their original ids. Admission deduplicates against
pending input ids and materialized `agent_message` ids. Replaying a committed
message neither creates a second receipt nor schedules a second wakeup.

`AgentTreeStore` owns this contract; backend storage implements the transaction.
A resolved provider steer alone is not a durable acknowledgement. Provider
interruptions follow the existing transcript recovery rules; the design does
not claim exactly-once inference across a remote-provider connection failure.

`agent` maps `agent_message` into provider input with an explicit source envelope.
A provider may require a user-role transport item for injected input; that wire
role does not make the message human-authored in the domain or UI. The envelope
identifies delegated observations and instructions as agent-originated context,
subject to the real user's task and constraints. It is never replayed as the
receiving agent's own assistant answer.

Edit, fork, compaction, and restore consume this same structured contract. Human
message editing and fork actions are not offered on receipts. Compaction retains
necessary receipt information as task context; it does not turn source text
into a human instruction. Historical receipts do not replay notification side
effects or resurrect the sending child.

## Product and gallery presentation

`web-ui` owns `AgentReceiptBlock`, built from `FunctionalBlock`. Both `web` and
`web-gallery` render the same implementation.

- A collapsed Bot-icon row names the sender and event: "UI implementation sent
  an update", "UI implementation completed", or "UI implementation failed".
- Click, Enter, or Space expands the row to reveal only the message or result.
  Sender ids, round identifiers, and timestamps stay in the structured data;
  the expanded body does not show them. Markdown uses the existing safe renderer.
- Receipt rows have no user bubble, human pending-steer controls, edit action,
  or message-level fork action. They are inspectable context, not user input.
- The receipt appears once in the transcript where it is incorporated into
  context. A closed-child lifecycle event updates the Agents chip/history; it
  does not create a second completion receipt.
- The user's message queue remains exclusively human-authored. Requesting,
  thinking, tool execution, and receipt rows retain their distinct meanings.

No receipt content is discarded merely to reduce visual noise. Collapsed rows
provide a quiet default while preserving the complete message for inspection.

## Acceptance

Use scripted providers and isolated storage/runner fixtures, never real models.

1. A busy parent receives an update and a completion before its next inference
   request; neither becomes a separate queued user turn.
2. Several messages admitted before one boundary are presented in order through
   one continuation, with distinct ids and no repeated acknowledgements.
3. Idle delivery wakes once; finalization/admission races lose no messages;
   user abort does not get undone by a late receipt.
4. Tool execution continues uninterrupted, and compaction preserves pending
   internal messages without exposing human pending-steer controls.
5. Restart after source close, target admission, or transcript materialization
   produces one receipt per completion round. A resumed child has a new receipt
   id; an old acknowledgement cannot consume the new round.
6. Both live-steer providers and providers requiring a subsequent request retain
   the agent source envelope and recover through persisted context.
7. Product and gallery verify collapsed/expanded updates, success/failure/abort,
   long content, repeated descriptions with distinct sender ids, and reconnect.
8. Genuine human sends and steers retain their behavior, metadata, queue controls,
   and cancellation semantics.
9. Agent inputs traverse the existing internal-steer and hidden-wakeup paths,
   producing `agent_message` blocks; human steers produce `steer` blocks. There
   is no agent-specific scheduler or duplicate receipt record.

## Implementation and verification

- `session/session.ts` owns admission, steering, hidden continuation actions,
  abort retention, and checkpoint flushes. `session/steer-queue.ts` owns the
  ordered pending records. Human pending-steer snapshots exclude agent input.
- `protocol/agent-message.ts` validates admitted and restored envelopes;
  `transcript/agent-message.ts` supplies the provider source wrapper.
- `subagent/supervisor.ts` supplies sender identity and completion timestamps,
  prevents closing with unread input, and observes terminal action failures.
  `store/tree-store.ts` identifies the exact completion rounds carried by a save.
- `__tests__/agent-messages.test.ts` covers durable busy batches, idle batching,
  live steering, finalization, abort/restore, deduplication, retry, and compaction.
  `subagent.test.ts` covers command delivery, nested failures, archive/reopen,
  and completion retries; memory and SQLite store tests cover atomic acknowledgement.
- `web-ui/agent/blocks/AgentReceiptBlock.vue` owns presentation. Gallery block
  specimens cover updates and all completion outcomes, expanded long content,
  and equal descriptions with distinct identities. The product renders the same
  rows through `AgentMessageVirtualBlock` from persisted transcript data.
