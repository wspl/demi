---
status: accepted
---

# A workflow's conversation spawns only the workflow's profiles

## Problem

A project's workflow declares the subagents its conversations may run. For
example, a review workflow has a `review` profile on a strong model and a
`research` profile its instructions keep for questions the user assigns.
The leading agent can still run `demi agent spawn` without `--profile`: the
child then takes the inherit profile, the parent's model, instructions and
Context, which the workflow never declared. The way of working the team
wrote down is one flag away from not applying, and a child the team did not
plan for spends the user's tokens.

## Decision

While a project's workflow is picked for the conversation, a spawn names
one of the workflow's profiles, and work none of them is for is the
spawning agent's own. A spawn without `--profile` fails before anything is
created, and says so:

```text
$ demi agent spawn < brief.md
error: workflow Review Team spawns only its profiles: review, research; do
other work in this session.
```

- The leading agent knows this before it spawns. The workflow's element of
  the instructions block, which now comes only with the workflow's
  `instructions`, comes whenever a workflow is picked, and the product adds
  the profiles and the rule after the team's text:

  ```text
  <workflow name="Review Team">
  Have review check every change before you report it.

  The only subagents you can spawn, with demi agent spawn --profile <id>:
  - review: Use to review a change before it is committed.
  - research: Use only when the user assigns a question to it.
  </workflow>
  ```

  The element follows the workflow's file as its instructions do, so an
  edited profile reaches the agent at its next request.
- A workflow with no profiles runs no subagents: its element says "You can
  spawn no subagents.", the spawn fails saying to do the work in this
  session, and `demi agent profiles list` says so on stderr.
- A picked workflow that cannot be used, its file gone or not reading, keeps
  its element too, which says so with the reason its spawns fail with: "You
  can spawn no subagents while this workflow cannot be used: it is no longer
  in the project's .demi/workflows/." Without it, the agent reads the
  element's removal as the rule's end.
- Global Workflow, the user's own profiles from Settings > Subagents, keeps
  the inherit profile: a user with no profile has no other way to spawn,
  and the user's settings are the user's own choice, not a team's.
- A workflow's profile without `model` still runs on the parent's model:
  the workflow declared that profile.
- `demi agent resume` without `--profile` still goes on with the profile the
  child runs on; it spawns nothing.
- `--profile`'s description in `demi agent spawn --help` says it is required
  while a workflow is picked.

`Tree::spawn_setup` takes the case in its `ConversationProfiles::Workflow`
arm: a spawn whose name is none fails there, where it now takes the inherit
profile. The instructions source (`backend-user-shard`'s `instructions`)
renders the profiles from the workflow it already reads. Nothing is stored
and no protocol changes; a conversation that has a workflow picked meets
the rule at its next request and spawn, and children already running go
on.

## Spike

None: the spawn's checks read the workflow at call time, so the rule is one
more check among them.

## Prior art

- Claude Code's Task tool takes a `subagent_type` from the agents the user
  and the project defined, beside a built-in general-purpose agent
  ([docs](https://docs.anthropic.com/en/docs/claude-code/sub-agents)), and
  the tool's description lists each agent with when to use it, so the model
  knows them before it calls. It has no setting that limits a session to a
  project's agents. Here a picked workflow is that limit, so it drops the
  general-purpose fallback, and the list goes in the workflow's element of
  the instructions block rather than a tool's description, which is the same
  for every conversation.

## Alternatives

- **Apply the rule to Global Workflow too.** A user with no profile could
  not spawn at all until they made one, and the inherit profile there
  bypasses no one's plan.
- **Spawn the workflow's first profile when none is named.** An agent that
  omits the flag gets a profile it did not choose, which for a reviewer on a
  costly model spends the most.

## Open questions

None.
