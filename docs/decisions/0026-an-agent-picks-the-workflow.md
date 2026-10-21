---
status: accepted
---

# An agent can pick the conversation's workflow

## Context and Problem Statement

A workflow applies to a conversation only when the user picks it in the
composer's Workflow menu, because a repository is often someone else's and
must not choose how an agent works by holding a file. An agent that sees a
task fits another way of working, or that the user asks to switch, has no
way to pick one: `demi agent workflows` only lists and checks them.

## Considered Options

* Keep the choice the user's alone.
* Let an agent pick a workflow, as the user would in the menu, with the
  user's permission.

## Decision Outcome

Chosen option: an agent picks it with the user's permission, because the
permission keeps the reason the choice was the user's: nothing a repository
holds applies without the user agreeing in this conversation.

* `demi agent workflows use <id>` picks a workflow of the conversation's
  directory for the conversation, and `use none` picks no workflow, exactly
  as the Workflow menu does. The menu shows the new choice at once.
* It needs the user's permission in each conversation, as changing a
  profile does; until the user answers, it waits.
* The choice applies as the menu's does: spawns use the new workflow's
  profiles from then on, and its instructions reach the leading agent from
  its next request.
* A workflow that does not read fails with why, and the choice stays as it
  was.

### Consequences

* Good, because an agent can move a conversation to the way of working its
  task needs without the user opening the menu.
* Bad, because one more permission prompt can interrupt the user.
