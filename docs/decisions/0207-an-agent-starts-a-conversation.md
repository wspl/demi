---
status: proposed
---

# An agent starts a conversation

## Problem

Alex's conversation is fixing the checkout page. Halfway through, Alex
decides the flaky login test deserves its own conversation, so this one
stays about the checkout. Today the agent cannot start one: Alex opens a
new conversation, picks the same project and model, and writes the brief
again, though the agent already holds the context the brief needs.

## Decision

An agent starts a new conversation of the user with a brief, and goes on
with its own work. The new conversation stands in the sidebar like one
the user started, runs at once, and is the user's to open, steer, move or
archive.

```text
Conversation "Fix the checkout total", a command of its agent:

  $ demi conversation new --title "Stabilize the login test" <<'EOF'
    The login test fails about one run in five on main: the session
    cookie is read before the redirect sets it. Find why and fix it.
  EOF
  Started “Stabilize the login test” (6f1c…).

Conversation "Stabilize the login test", its main agent's transcript:

                Conversation: Fix the checkout total
          ┌──────────────────────────────────────────────┐
          │ The login test fails about one run in five   │
          │ on main: the session cookie is read before   │
          │ the redirect sets it. Find why and fix it.   │
          └──────────────────────────────────────────────┘
```

- **What it starts with.** The new conversation takes the starting
  conversation's place and settings: its workspace, its Host and
  directory (its project, and the worktree it works in, when it has
  one), its model with its thinking and tier, and its workflow choice.
  `--project <id>`, from `demi conversation projects`, starts it in
  another project's directory instead; a workflow is a file of its
  project, so another project's conversation takes the user's Subagents
  settings, unless subagents were off. Its agent can still move it with
  `demi host move`.
- **The brief.** The message comes on standard input, as `send` takes
  it, and is the conversation's first message: an agent's message from
  the starting conversation, as `demi conversation send` delivers one,
  so the bubble names the conversation it came from and links to it,
  and the new agent can report back with `send` when the brief asks it
  to. Nothing reports back by itself.
- **Its title.** `--title` names it; without one, it is titled from the
  brief as a user's first message titles a conversation.
- **The user's view.** Nothing switches the user's page: the
  conversation appears at the top of the sidebar, running, as a
  conversation another page started does. The command answers once the
  brief is taken, with the title and the id, and the agent links the
  conversation in its reply as `[Stabilize the login test](conversation:6f1c…)`,
  which opens it, as `agent:4` opens a subagent.
- **Permission.** Starting a conversation belongs to `conversation.use`
  (0205), asked once per conversation with listing, reading and sending;
  its description says the agents may also start conversations.
- **Limits.** None: the user sees every conversation run in the sidebar
  and stops or archives any.
- **A failure.** What cannot send a message, such as a side question's
  agent, is refused before anything is created. A step that fails once the
  conversation exists names it, so the agent sends the brief again with
  `send` instead of starting another.

The command is a leaf of the `conversation` group in
`crates/backend-user-shard/src/conversation/organize.rs`. It creates the
record through `create_started`, the creation the page's request makes
(`creation.rs`), with the starting conversation's target and settings,
then admits the brief as `send` admits a message. Nothing new is stored
and no protocol between the page and the backend changes; the web app
learns the conversation from the list it already follows. The page that
renders the link is the one the server serves, so no older page meets it.

```rust
/// The input of `demi conversation new`.
struct NewArgs {
    /// The brief, the conversation's first message.
    message: String,
    /// Its title; generated from the brief when absent.
    title: Option<String>,
    /// The project it starts in; the starting conversation's place when absent.
    project: Option<ProjectId>,
}
```

## Spike

Whether an agent's message admitted into a conversation that never ran
starts its first turn, as it wakes an idle one: on the development setup,
an agent started a conversation, and its brief opened the first turn at
once (`wakeup.opened_turn`, cause `agent_message`), which ran the brief's
command and answered.

## Prior art

- OpenClaw's `sessions_spawn`
  ([session tool](https://docs.openclaw.ai/concepts/session-tool/),
  [`spawn-plan.ts`](https://github.com/openclaw/openclaw/blob/f9478d593c96c955db06923f4aa6367fb2825b61/src/agents/spawn-plan.ts))
  starts a child session from an agent's call and returns at once; the
  child's result is announced back to the requester. Its `session` mode
  keeps the child as a thread of the chat it came from, and the
  configuration caps how many children an agent has and how deep they
  nest. Demi's conversation is the user's own, a peer in the sidebar
  rather than a child, it reports back only when the brief asks, through
  `demi conversation send`, and nothing caps how many there are.
- Demi's subagents (`demi agent spawn`) are that child: they work inside
  the conversation, and the user reads them in its view. A new
  conversation is for work the user wants to follow on its own.

## Alternatives

- **A subagent**: its work stays in the starting conversation's view and
  ends with it, which is what the user wants out of the way.
- **A user's message as the brief**: the new conversation would not say
  which conversation started it, and its agent could not answer it.
- **Switching the user's page to it**: it would take the user out of the
  conversation they are reading.
- **Its own permission**: it acts on the user's conversations as `send`
  does, and one more card per conversation says nothing new.
- **No permission**, as a subagent asks none: a new conversation runs the
  user's model on its own, and the user is asked once which conversation
  may do that for them.

## Open questions

None.
