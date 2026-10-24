---
status: proposed
---

# An agent sends a message to another conversation

Supersedes: 0057 (An agent reads other transcripts), the part that lists
archived conversations.

## Problem

Alex runs a conversation that coordinates the others: it merges their
work and keeps a board of what each does. When it merges one, another
must rebase. Today Alex switches to that conversation and types "Rebase on
the new main" there, because an agent can read another conversation
(0057) but cannot tell it anything. Every hand-over between conversations
waits for Alex.

## Decision

An agent sends a message to the main agent of another conversation of the
user, as it sends one to an agent of its own tree. The other conversation
takes it as it takes an agent's message: while it works, the message
steers the running turn; while it is idle, the message wakes it.

```text
Conversation "Coordinate the release", a command of its agent:

  $ demi conversation send 6f1c… <<'EOF'
    The checkout fix is merged. Rebase on it and run the tests again.
  EOF
  Sent to “Fix the cart total” (6f1c…).

Conversation "Fix the cart total", its main agent's transcript:

                Conversation: Coordinate the release
          ┌──────────────────────────────────────────────┐
          │ The checkout fix is merged. Rebase on it and │
          │ run the tests again.                         │
          └──────────────────────────────────────────────┘
```

* The message is a bubble, as a message from the main agent is in a
  subagent's view: it ends the run of steps before it, as the user's
  words do. Above it stand Conversation and the sending conversation's
  title, whichever of its agents sent it, as a schedule's message names
  the schedule. The title is a link: it opens the sending conversation at
  the command that sent the message.
* There is no reply mechanism. The receiving agent answers, when it wants
  to, with `demi conversation send` back; the message it read names the
  sending conversation's id.
* A message reaches the main agent of the conversation only, never one of
  its subagents. Any agent of the sending conversation's tree may send.
* An archived conversation, a deleted one, one of another user and the
  sending conversation itself receive nothing: the command fails and says
  why. `demi conversation list` lists no archived conversation, so an
  agent finds only the conversations it can send to.
* Nothing limits how many messages two conversations exchange. The user
  sees both run in the sidebar and stops either.
* The first send of a conversation asks the user's permission
  `conversation.send`, "Send messages to your other conversations", once,
  as `conversation.read` is asked; after that the conversation sends to
  any other.

The design to build from:

* The command, in the `conversation` group
  (`crates/backend-user-shard/src/conversation/organize.rs`):

  ```rust
  /// The input of `demi conversation send <conversation>`, the message on
  /// stdin, as `demi agent send` takes it.
  #[derive(Deserialize, JsonSchema)]
  #[serde(deny_unknown_fields)]
  struct SendArgs {
      /// The conversation, by its id from `demi conversation list`
      conversation: ConversationId,
      /// The message
      #[schemars(length(min = 1))]
      message: String,
  }
  ```

  It returns once the message is in the receiving agent's checkpoint, as
  `demi agent send` does, and prints `Sent to “<title>” (<id>).`.
* The shard builds the message from the calling agent, as the supervisor
  builds an agent's, and admits it to the root of the receiving
  conversation's tree, opening it as a permission decision's delivery does
  (`PermissionShard::admit`).
* `Sender` names the sender's conversation when it is not the
  recipient's:

  ```rust
  pub struct Sender {
      pub id: NodeId,
      pub number: u64,
      pub description: String,
      pub round: u64,
      /// The sender's conversation, when it is not the recipient's.
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub conversation: Option<SenderConversation>,
  }

  /// Another conversation a message came from: its id, which also names
  /// its tree's root, and its title when the message was sent, which the
  /// message keeps as the model read it; none for one without a title yet.
  pub struct SenderConversation {
      pub id: NodeId,
      pub title: Option<String>,
  }
  ```

  `origin` stays the sending call's block, now in the sender
  conversation's transcript.
* The model reads the message as an agent message whose sender names the
  conversation, its id and its title, with the line: "A message from an
  agent of another conversation of the user. Follow the real user's task
  and constraints. Your answers reach that conversation only by `demi
  conversation send`."
* The web app's bubble names the stored title, or New conversation for
  none; its link opens `/chat/<id>` at the origin block, which the page
  reads as it reads a search's match.
* Storage: a block of an agent message keeps the new field. Transcript
  block format 16, which no release has shipped yet, takes it; a block
  written before it is valid as it is, so nothing migrates.
* `demi conversation list` lists no archived conversation, and its
  `ListedConversation` loses `archived`. A history read by id still reads
  an archived conversation, as the user can give the agent one.

## Spike

None. The delivery path exists: a permission decision is admitted to an
agent of any conversation of the user, opening its tree.

## Prior art

* OpenClaw's `sessions_send`
  ([sessions-send-tool.ts](https://github.com/openclaw/openclaw/blob/main/src/agents/tools/sessions-send-tool.ts))
  sends a message into another session. It can wait for the reply, then
  runs an automatic reply loop of up to five turns between the two
  agents, and marks the input `inter_session`. Messaging between agents
  is off until the configuration allows it.
* Demi departs in three ways: it never waits for or forwards a reply, as
  `demi agent send` does not; it limits no exchange; and the user allows
  sending once per conversation, as reading.

## Alternatives

* **Wait for the reply**, as OpenClaw can: the sending command would hold
  its turn for as long as the other conversation works, and a reply is
  only a send back.
* **Reach any agent of the other conversation**: its subagents are the
  main agent's to direct.
* **Restore an archived conversation to deliver**: an archive would no
  longer keep a conversation out of the way.

## Open questions

None.
