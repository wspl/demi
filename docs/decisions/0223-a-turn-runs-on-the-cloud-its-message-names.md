---
status: proposed
---

# A turn runs on the Cloud its message names

## Problem

A group's conversation runs each person's request on that person's Cloud,
where their sign-ins are. alex asks for a report and the turn starts on
alex's Cloud; while it runs, sam asks for something else. Today the program
can only `PATCH {cloud}` to sam's Cloud and then send sam's message: the
move takes effect at the main agent's next command, so the rest of alex's
turn runs on sam's Cloud, with sam's sign-ins, where the files it made are
missing. The program cannot queue sam's message with sam's Cloud either:
a queued message has no Cloud, and the program cannot hold the move until
alex's turn ends without keeping a second queue of its own.

A conversation also always works in `~`, though a team's bot works in the
folder all Clouds share.

## Decision

The message that starts a turn names its Cloud, and the move happens as the
turn starts.

* `SendMessage` gains `cloud`, one of the integration's Clouds; none keeps
  the conversation where it is. The message joins the queue as today, and
  the queue entry keeps its Cloud: the lane's `SendMessage` and
  `QueuedMessage` gain `device`, an optional device id, which the checkpoint
  keeps with the queue.
* When the session starts a turn from a message that names a device, it
  writes the message and asks the product to move the conversation there
  (`HostResolver::move_to`) before the turn's first request, through the
  same move the web app's project change and `PATCH {cloud}` use; a move
  to where the conversation works changes nothing. The execution context
  source tells the agent its new Host, as after any move. A move that
  fails, such as to a Cloud being deleted, fails the turn, which ends with
  an error before any request (`move_failed`).
* A steer that names a Cloud other than where the running turn works is
  refused with `409 cloud_differs`, so the program queues it instead. A
  steer into an idle agent starts a turn, and moves as a message does.
* `PATCH {cloud}` moves the conversation at once, whether or not a turn
  runs, as the web app's move does: a running turn's next commands run on
  the new Cloud.
* A turn a wakeup or a subagent's message starts runs where the
  conversation works, which is where the turn that left that work ran. A
  subagent keeps its own Host (0287).
* The conversation's `cloud` and `directory` say where it works now, and
  the queue's entries (0220) say each one's Cloud.

### The directory

`CreateConversation` gains `directory`, a path on its Clouds, `~` by
default, such as the shared folder's `/workspace`; `PATCH {directory}`
changes it under the same rule as the Cloud. A directory other than the home
is a project of the integration's workspace, which the web app lists
nowhere, as every move to a directory makes one. A move keeps the directory,
which every Cloud of the integration has when it is in the home or the
shared folder; on a Cloud that lacks it the conversation's directory is
missing, as a project's is on a device that lacks its folder.

## Spike

* `PATCH {cloud}` moves through `move_to` with a device target
  (`backend-user-shard/src/integrations.rs`), the move the session asks
  for. A move while the conversation's turn runs is one `demi host move`
  makes already.
* The session writes a message and runs its turn in one action
  (`agent-session/src/worker.rs`), where the move goes between the two.

## Prior art

* GitHub Actions binds a job to the runner its `runs-on` names when the job
  starts, and never moves it.
* Kubernetes binds a pod to a node once, at scheduling.

Both place work when it starts, as this does.

## Alternatives

* **A Cloud per turn, with no conversation-wide Cloud**: every part of Demi
  that reads where a conversation works, its panels, its files, its
  attached Hosts, would read a turn's instead.
* **Moves held by the program until the turn ends**: a second queue, whose
  entries the web app's queue routes (0220) would not see.

## Open questions

None.
