---
status: proposed, superseded in part by 0077
---

# A call that waits for the user's permission holds until the user allows it

Supersedes, in part: 0052 (a call waits for its command), where a call
returns after 30 seconds while its command still waits for the user's
permission.

## Problem

An agent runs `demi automation create …`, which needs the user's
permission, and a card asks the user. The user is reading something else
and answers after a minute. Thirty seconds in, the call returns to the
model with the command still running and a line that it waits for the
user's permission. The model now holds the turn while nothing it asked for
has happened: it may run the same command again, whose card replaces the
first, or go on as if it had been allowed.

## Decision

**The call holds while the user decides.** While a command waits for the
user's permission, its call does not return and its 30 seconds do not run.
Once the user allows it, the command goes on and the 30 seconds count from
then, as for any command.

**Anything but Allow denies it.** The user denies a waiting request with
Deny, and with any other action on the agent whose call waits:

* a message to that agent, queued, sent now or as a steer;
* Edit and Send, Regenerate or Retry on that agent, once the session takes
  it;
* Stop, of the conversation or of that agent, or of a subagent above it,
  which stops it;
* stopping the waiting command itself.

A denied request is decided as Deny decides it: the card shows it denied,
and the call returns as any call does, when its command ends, with exit 1
and the line it has today, `the user denied this conversation permission
to {action}; the command was not run`. A message that denies does not
return the call itself, a send now included: it arrives after that
result.

```text
  $ demi automation create …        waiting for your permission
  [Allow] [Deny]
                                    the user sends "use 9 am instead"
  exit 1  the user denied this conversation permission to create an
          automation; the command was not run
  user    use 9 am instead
```

The design to build from:

* `agent-tools`: a call's window does not run while its job awaits the
  user (`ShellEnvironment::awaits_user`); it starts again, whole, from the
  moment the job goes on. What arrives meanwhile returns the call once
  the user has decided, as an arrival does during any call.
* `agent-session`: the decision's message answers the call that waited,
  so it is no arrival that ends a call's window.
* `backend-permissions`: each action of the user above decides every
  waiting request of that agent's calls as `Denied`, stored as the Deny
  button stores it. A send now is decided once the session has it, and no
  message returns the call: `agent-tools` ignores what arrives while its
  job awaits the user.
* The waiting line, `PortRequest::AwaitsUser`'s `line`, goes: no call
  returns while its command waits, so no result shows it.

Unchanged: a credential request, which waits for the user to fill a card
and returns its call as today; a request whose call stops for another
reason, a parent's abort or a Cloud reset, which stays on its card. A
request stored before this change was held by a backend that has since
restarted, which ended its call, so nothing older meets the new rule.

The conversation is busy while a call holds, as during any call: an
automation that wakes it while the user has not answered waits behind the
card, as it waits behind any turn.

## Spike

None.

## Prior art

* The Codex CLI, open source, holds a command on its approval request
  until the user answers, with no time limit; its "No, and tell Codex what
  to do differently" denies the command and takes the user's words
  ([approval requests](https://github.com/openai/codex/blob/main/codex-rs/tui/src/bottom_pane/approval_overlay.rs)).
* Claude Code, whose source is not open, does the same, as its
  documentation says
  ([permissions](https://docs.claude.com/en/docs/claude-code/iam)).

## Alternatives

* **Return after 30 seconds with a waiting line, as today.** The model
  gets the turn back while nothing it asked for has happened.
* **Keep the request open while a message reaches the model.** The model
  reads a message beside a command that may still run, and must guess
  whether the user's words answer the card.

## Open questions

None.
