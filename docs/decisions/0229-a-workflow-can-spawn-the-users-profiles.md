---
status: accepted
---

# A workflow's conversation can spawn the user's profiles too

Supersedes: 0112 (A workflow's conversation spawns only the workflow's
profiles), the part that keeps the user's profiles out of a workflow's
conversation. Its other rules hold: a spawn names a profile, and a workflow
has no inherit profile.

## Problem

Alex keeps a `translate` profile in Settings > Subagents, on a cheap model,
for every project. A project's Review Team workflow declares `review` and
`research`. Once Alex picks Review Team for a conversation, `demi agent
spawn --profile translate` fails: the workflow's profiles replace the
user's. Alex either copies `translate` into the project's file, where the
whole team gets it, or switches the conversation back to Global Workflow
and loses the team's profiles. A team that wants its conversations to run
only what it planned has nothing to say so, since that is the only behavior
today.

## Decision

A workflow's conversation spawns the workflow's profiles and the user's
enabled profiles, unless the workflow's file turns the user's off:

```yaml
name: Review Team
user_profiles: false
profiles:
  - id: review
    description: Use to review a change before it is committed.
```

- `user_profiles` is optional and `true` by default. With `false`, the
  workflow behaves as 0112 built it.
- A name that both have is the workflow's: in that conversation `--profile
  review` spawns the workflow's `review`, and the user's is not listed. The
  team's definition is the one its instructions were written for.
- The user's part follows Settings > Subagents as Global Workflow does: a
  disabled profile is not offered, and with the Subagents switch off none of
  the user's are. The switch never stops the workflow's own profiles.
- A spawn without `--profile` still fails; its message names every profile
  it takes, the workflow's first:

  ```text
  $ demi agent spawn < brief.md
  error: workflow Review Team spawns only named profiles: review, research,
  translate; do other work in this session.
  ```

- A child of the user's profile hears the workflow's
  `subagent_instructions` before its profile's own, as the workflow's
  children do: they say how every subagent of the conversation works with
  its lead. The field's help says "every subagent of the conversation".
- `demi agent profiles list` lists the workflow's profiles first, in the
  file's order, then the user's in name order; its SOURCE column says
  which is which, as it already does. `demi agent workflows check` adds a
  `User profiles: yes` or `no` line.
- The workflow's element of the instructions block lists the workflow's
  profiles, and with `user_profiles` on adds one line, never the user's
  names, which stay out of every prompt as 0112's module comment says:

  ```text
  The subagents you can spawn, with demi agent spawn --profile <id>:
  - review: Use to review a change before it is committed.
  - research: Use only when the user assigns a question to it.
  - The user's own profiles, which demi agent profiles list shows.
  ```

  A workflow with no profiles of its own lists only the last line; with
  `user_profiles: false` the heading says "The only subagents", and a
  workflow with no profiles says "You can spawn no subagents." as now.
- With the Subagents switch off, a spawn that names one of the user's
  profiles fails saying the switch is off, never that the name is unknown,
  and `demi agent profiles list` says on stderr that the user's profiles are
  off; a resume takes them, as under Global Workflow.
- The composer's context indicator, which counts the cost of the profiles
  the conversation can spawn, counts the user's too.

Design:

- `agent-tools`' `Workflow` gains `user_profiles: bool`
  (`#[serde(default = "yes")]`), and `WORKFLOWS_HELP` its line.
  `ChosenWorkflow` carries it and the workflow's `subagent_instructions`.
- `Tree::spawn_setup`'s `Workflow` arm looks for the name in the workflow's
  profiles, then, with `user_profiles` and the switch on, among the user's,
  through the same code the `Settings` arm uses, with the shared
  instructions put first.
- `WorkflowOption` on the page gains `userProfiles`, and
  `workflowProfiles` returns the union for it.
- Nothing is stored and no protocol between Host and backend changes. A
  workflow file of today reads unchanged and now offers the user's profiles
  too; the page and the backend are one build, so no page meets a backend
  without `userProfiles`.

## Spike

None: the lookup is one more list in a check that already reads both at
call time.

## Prior art

- Claude Code offers a session the agents of the project's
  `.claude/agents/` and the user's `~/.claude/agents/` together, and when
  names collide the project's wins
  ([docs](https://docs.anthropic.com/en/docs/claude-code/sub-agents)). It has
  no setting that keeps a project's sessions to the project's agents; this
  record keeps 0112's behavior as that setting.

## Alternatives

- **A switch per conversation, in the Workflow menu.** The team cannot say
  what its way of working needs, and each user meets one more choice in
  every conversation.
- **The user's name wins a collision.** The workflow's instructions name
  `review` meaning the team's reviewer; the user's would run in its place
  unnoticed.
- **List the user's profiles in the instructions block.** Every edit in
  Settings would change every workflow conversation's transcript, which is
  why 0112 kept them out.

## Open questions

- Whether this repository's own workflows, `.demi/workflows/parallel.yaml`
  and `slots.yaml`, turn the user's profiles off to keep their current
  behavior.
