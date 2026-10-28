---
status: proposed
---

# A skill source is global or a workspace's

Supersedes: 0142 (An account holds workspaces, each with its own
conversations and projects), the part that every workspace shares skills.

## Problem

Alex keeps a Work workspace for his job and a Personal one for himself. His
employer's skills, from `acme/internal-skills`, help only with work, yet
every Personal conversation offers them to the model and every Host his
Personal conversations use receives their files: skills are the account's,
the same in every workspace.

## Decision

Each skill source is in one of two places: **Global**, offered in every
workspace, or **a workspace**, offered only in that workspace's
conversations. A conversation is offered the skills that are on in Global
and in its own workspace.

Settings › Skills lists the two places as two groups, Global and the
workspace of the tab Settings opened in, each with its own Add Source…:

```text
Skills
Packaged workflows the agent can follow.

Global                                             [Add Source…]
Available in every workspace.
┌──────────────────────────────────────────────────────────────┐
│ anthropics/skills                          [on]   ↻   🗑      │
│   pdf · docx · xlsx                                          │
└──────────────────────────────────────────────────────────────┘

[▣] Work                                           [Add Source…]
Available only in this workspace.
┌──────────────────────────────────────────────────────────────┐
│ acme/internal-skills                       [on]   ↻   🗑      │
│   deploy · release-notes                                     │
└──────────────────────────────────────────────────────────────┘
```

* Every source that exists today is Global, so nothing changes until a
  source is added to a workspace.
* A source is added where its Add Source… is, and stays there. To move it,
  the user removes it and adds it in the other place.
* A repository is a source in one place. Adding one that another place
  has is refused, naming the place; a repository two workspaces need goes
  in Global.
* Deleting a workspace removes its sources with it.
* A conversation moved to another workspace is offered that workspace's
  skills from its next turn.

**Names.** Two skills that are on never share a name, in any place, as
today: a skill's directory on a Host is named after it, and a Host may run
conversations of several workspaces. A new skill whose name a skill that
is on has starts off, with *Name taken* naming the other's repository and
place.

**The agent.** `demi skills` acts on what its conversation is offered:

* `list` lists Global's sources and the conversation's workspace's, with a
  column naming each one's place.
* `add` adds to Global, as today; `--workspace` adds to the conversation's
  workspace.
* `update`, `remove`, `enable` and `disable` find the source by its
  repository among the two places' sources.
* Manage Skills, the permission, says that Global skills reach every
  conversation and a workspace's skills reach that workspace's.

**Hosts.** A skill is installed where the conversations offered it run.
A Host receives a skill's directory when the first job of a conversation
offered that skill runs there, as a Host first used receives its skills
today:

* A Global skill reaches every Host the user's jobs run on.
* A workspace's skill reaches only the Hosts its workspace's conversations
  run on: Work's `acme/internal-skills` never reaches the Mac that only
  Personal conversations use.
* A conversation moved to another workspace takes its new workspace's
  skills to a Host at its next job there.
* Turning a skill off or removing its source removes its directory from
  every Host that has it, as today.

The path of a skill's directory names no workspace, so a Host that serves
two workspaces keeps one copy of a skill both are offered.

The design to build from:

* A source's value gains its place, `global` or a workspace's id; a value
  without one is Global. Its id stays its repository's, so one repository
  is one source.
* A Host directory gains the workspace whose jobs install it, none for
  every job: `plugin_directories` gains `workspace_id`, which a workspace's
  deletion clears with its rows.
* The catalog of a conversation takes the sources of Global and of the
  conversation's workspace, and a job installs on its Host the skills of
  that catalog that the Host lacks.
* The page state carries the sources of every place; the page shows Global
  and its tab's workspace.

## Spike

None: a conversation's workspace is known wherever its catalog is made.

## Prior art

* **Claude Code** reads skills from the user's directory, offered in every
  project, and from a project's `.claude/skills`, offered in that project:
  two places, and a conversation takes both.
* **VS Code** keeps user settings and a workspace's settings, and enables an
  extension globally or for one workspace only.

Demi takes the same two places, with the workspace as the narrower one.

## Alternatives

* **A workspace chooses whether it shares the account's skills or keeps its
  own list**, as a project chooses how it keeps saved messages: a workspace
  that keeps its own would have to copy or give up every shared skill, and
  switching back would ask what becomes of its list.
* **Move To on a source's menu**: saves a removal and an addition, which a
  source's fetch makes quick anyway.

## Open questions

None.
