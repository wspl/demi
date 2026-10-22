---
status: accepted
---

# An agent reads other transcripts

## Summary

`demi conversation history` reads any transcript, not only the agent's
own: an agent of this conversation's tree by its number, or an agent of
another conversation of the user. `demi conversation list` finds the user's
conversations. `demi agent show` keeps to an agent's state, and the history
commands are the one way to read what any agent said.

## Motivation

* A subagent ended without sending its report. Its parent can see only what
  `demi agent show` keeps, the titles of its last commands and its last
  text; the history commands read only the transcript of the agent that
  runs them.
* A user asks, in a new conversation, to go on with what an earlier
  conversation decided. The agent can neither list the user's conversations
  nor read one.

## Guide-level explanation

```text
$ demi conversation history search --agent 1099 measured
$ demi conversation history read 4210..4260 --agent 1099
$ demi conversation list cache
ID                                    TITLE                  PROJECT  CHANGED
6f1c…                                 Cache the index        shop     2026-10-21T18:04:12Z
$ demi conversation history read ..40 --conversation 6f1c…
```

* `--agent <n>` reads an agent of this conversation's tree, the main agent
  included, with no permission asked.
* `--conversation <id>` reads another conversation of the user, its main
  agent unless `--agent` names one of its agents. The first list or read of
  another conversation asks the user once in the calling conversation, as
  any permission does; after that the conversation reads any other.
* `demi agent show` says what an agent is doing: its execution state, its
  idle time, its recent commands, how it closed. It no longer prints the
  agent's last text; its output names the history read that does.

## Reference-level explanation

### Commands

```rust
/// Which transcript a history command reads; none is the caller's own.
#[derive(Deserialize, JsonSchema)]
struct Target {
    /// An agent, by its number: of this conversation's tree, or of the
    /// conversation `conversation` names
    agent: Option<u32>,
    /// Another conversation of the user, by its id; its main agent unless
    /// `agent` names another
    conversation: Option<ConversationId>,
}

// SearchArgs, ReadArgs and OpenArgs each gain `#[serde(flatten)] target: Target`.

/// The input of `demi conversation list`.
#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Words the title or the messages hold, as the app's Search finds them
    words: Option<Vec<String>>,
    /// The page, from 1; twenty conversations to a page
    page: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedConversation {
    id: ConversationId,
    title: Option<String>,
    project: Option<String>,
    archived: bool,
    changed_at: Timestamp,
}
```

* `list` prints a table of id, title, project and the time of the last
  change, newest first, the archived ones included and marked; `--json`
  prints `Vec<ListedConversation>`; stderr names the next page. With words
  it lists at most the fifty results the app's Search returns, and stderr
  says so when there are more.
* A target that does not exist, or a conversation of another user, fails
  with `no transcript <target>`, exit 1.
* `--from user` and `--from agent` keep their meaning relative to the agent
  read: its user or parent, and itself.
* Output, paging and the bounds of `read` and `open` are today's. A read of
  another transcript starts by naming it, and every command its output
  suggests, a page, an open or a cut, carries the same `--agent` and
  `--conversation`.

### Permission

* `list`, and any history command with `--conversation`, need the
  permission `conversation.read`, "Read your other conversations", asked
  once in the calling conversation as other permissions are; naming the
  caller's own conversation asks too.
* `--agent` alone needs none.

### State

All reads are from the backend: the conversations' table for `list`, each
conversation's database and its search index for the history commands. A
Host is reached only as today's `open` reaches one: to write an opened
attachment to the caller's scratch directory, and to read the output of a
command still running, from the Host it runs on.

### `demi agent show`

* Its "Last assistant text" lines are removed.
* Its last line names the read of the agent's latest blocks:
  `More: demi conversation history read <from>.. --agent <n>`.

## Upgrade and version skew

Old transcripts are read as they are. The commands' manifest gains the
field that names the argument a permission depends on; a runner older than
this backend rejects the manifest, so the runner and the backend upgrade
together, as for every manifest change.

## Performance

* A read or a search of another transcript costs what one of the caller's
  own does: bounded pages from one database.
* A list is one bounded query of the conversations' table; with words, the
  query the app's Search runs.

## Drawbacks

* What another conversation holds enters this one's transcript, and goes to
  this conversation's model provider: the reason for the permission.

## Rationale and alternatives

* `demi agent history <id>`: a second way to read a transcript beside the
  history commands, for agents only, and none for other conversations.
* Keep the last text in `agent show`: the same text read in two places.

## Prior art

* Codex has `list_threads` and `read_thread`, a page of turns newest first,
  with outputs cut to a few thousand characters.
* Claude Code's Agent SDK has `list_sessions`, `get_session_messages`,
  `list_subagents` and `get_subagent_messages`, paged by offset, for a host
  rather than the model.
* OpenHands' server lists conversations and searches a conversation's
  messages a page at a time; Cline searches every session's text.
* Demi departs in asking the user before another conversation is read; none
  of them asks per conversation.

## Unresolved questions

None.
