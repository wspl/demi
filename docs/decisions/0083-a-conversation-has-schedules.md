---
status: proposed
---

# A conversation has schedules, which the agent sets and wake it

Supersedes: 0010 (Automation tasks in a repeating cycle, delivery to a busy
conversation, and runs that manage automations), the parts that name a new
conversation each run, the editor and the Automations page.

## Problem

Automations belong to the account. Alex asks the agent in a conversation to
check the CI every morning; the agent makes an automation, which lives on a
page of its own, with its own editor, a model, a device and a project for
the new conversations it may start, a list of runs, and conversations filed
out of the sidebar. To see what wakes the conversation, Alex leaves it for
that page. Almost every automation wakes the conversation it was made in.

## Decision

An automation becomes a **schedule**, and a schedule belongs to one
conversation: it wakes that conversation and nothing else. The agent
creates, changes and deletes the conversation's schedules with
`demi schedule`; the user sees them, turns them off and on, and deletes
them. Nothing else changes in what a schedule can do.

### What the user sees

A chip with an alarm clock and how many, *3 Scheduled*, stands above the
composer, beside Running and Agents, while the conversation has schedules,
as Running says how many commands run. It opens the schedules as the app's
own menu, upward, and as a sheet on a phone: a list, where the user turns a
schedule off and on with a switch, as the Clock app's alarms are, and
deletes it; the agent does the rest.

```text
 ┌──────────────────────────────────────────────────────────────────┐
 │ [cal] Weekly report  Every week · Mon 10:00     in 2 days   (●)  │
 │ [bin] CI on main     When output changes · 5 min  3 min ago (●)  │
 │ [tmr] Remind me      Once · Thu 09:00                       (○)  │
 └──────────────────────────────────────────────────────────────────┘
  [🤖 2 Agents]  [▶ 1 Running]  [⏰ 3 Scheduled]
 ┌──────────────────────────────────────────────┐
 │ Ask anything…                                │
```

* A row's icon is its trigger's kind: a calendar for a time schedule that
  repeats, by a cycle, an interval or cron; a timer for one that wakes
  once; binoculars for a command's output that changes; the webhook for a
  URL.
* On a computer a row is one line: the name, the trigger in words and, in
  the warning color, why its last wake failed or was skipped. At its end
  stands its switch, and before it, lined up with the other rows', when it
  next wakes the conversation, as a time from now; for a change watch or a
  URL, when it last did. A schedule that is off is dimmed and has no time.
  Over the pointer's row, Delete takes the time's place, as Mail shows a
  row's actions over its date.
* On a phone a row has up to three lines: the name with the time, the
  trigger, and the reason, and the switch at its end. Swiping a row to the
  left lays Delete bare.
* The switch and Delete ask nothing, as deleting a queued message does
  not, and leave the menu open; with no schedule left, the chip and its menu
  go.
* In an archived conversation, whose schedules wake nothing, every row is
  dimmed, with no time, its switch disabled and no Delete.
* A message a schedule sent shows "Schedule" and its name above it in the
  transcript, as it shows "Automation" today, and the name opens the menu.
* The sidebar's bolt beside a conversation becomes the alarm clock.

### What goes

* The sidebar's Automations entry, its page and its count, the editor and
  its dialogs, and Run Now.
* A new conversation each run, with the model, the device and the project
  it named. Conversations filed under an automation's run join the sidebar.
* A URL trigger's check of Stripe's signatures, whose signing secret the
  user pasted in the editor.
* The run's bar under a filed conversation's header.

### What stays

The triggers and their rules, as 0010 and the code have them: once, a
repeating cycle with tasks in it, every so many minutes, cron, a command's
output that changes, and a URL with its secret and its daily limit; the
time zone; queue or steer when the conversation is busy. A change watch
runs on the conversation's Host, in its directory. A schedule wakes
nothing while its conversation is archived, and wakes it again, counting
from then, once it is not.

A once schedule is deleted after it wakes the conversation.

### The design to build from

* One concept, one name: `automation` becomes `schedule` everywhere: the
  types (`ScheduleId`, `Trigger`, `ScheduleRun`), the tables, the
  permission (`schedule.manage`), the command, the message mark a user
  block carries (`schedule`), and the words. Stored data moves by
  migration: the control database's tables, and each conversation
  database's user blocks.
* The conversation's metadata carries its schedules, read with it and
  changed on its channel:

  ```ts
  interface ConversationSchedule {
    id: string
    name: string
    trigger: Trigger
    timeZone: string
    paused: boolean
    nextAt: Timestamp | null
    lastRun: { at: Timestamp; state: RunState; note: RunNote | null } | null
  }
  ```

* An archived conversation's schedules wake nothing. That is read from the
  conversation's archived state, never written to a schedule's own `paused`,
  which only the user and the agent set and which the metadata carries; the
  page shows the archive's hold from the conversation's archived state.
* The conversation summary gains `scheduled`, whether the conversation has a
  schedule, which the sidebar's alarm clock shows. It is read from the
  schedules where the summary is made.
* The page writes `PATCH /conversations/:id/schedules/:schedule` with
  `{ paused }` for the switch, and `DELETE` on the same path. Every
  `/automations` route goes; `/hooks/<id>` stays.
* `demi schedule list | show | create | edit | run | delete` acts on the
  conversation it runs in only: `--conversation`, `--new`, `--device`,
  `--directory`, `--model` and `--effort` go. Nothing shows a URL
  trigger's secret again, so `show` prints its address only, `create` prints
  the secret once, and `edit --new-secret` makes a new one at once, which it
  prints once; the old one stops working.
* The product state loses `automations`, `automationsUnread` and the
  conversation summary's filed run.
* Migration: an automation that wakes an existing conversation becomes that
  conversation's schedule. One that starts a new conversation each run is
  deleted with its runs; the conversations its runs started stay. A URL
  secret is sealed again under the schedule's name; the Stripe signing
  secret goes; the permission to manage automations becomes the permission
  to manage schedules; the runs that were skipped for a conversation gone
  or a model not chosen, which no reason words now, go.

### The agent's words

`demi schedule`'s description, before and after:

```text
Before: The user's automations: the agent started on a repeating cycle
with tasks in it, once, every interval, by cron, when a command's output
changes, or when a URL is called, in a new conversation each run or in an
existing one. List, show, create, change, run now and delete them.

After: This conversation's schedules, which wake it: once, on a repeating
cycle with tasks in it, every interval, by cron, when a command's output
changes, or when a URL is called. List, show, create, change, run now and
delete them.
```

`create` loses "Without --conversation or --new it wakes this
conversation."; its other words stay.

The other words follow the same two rules: what belongs to a new
conversation, the user's Run Now and the schedule's page goes, and
`automation` reads `schedule`. `edit` takes `--new-secret`, "Make a new
secret for a URL trigger and print it once; the old one stops working at
once." A message a schedule sends begins "Schedule “{name}”, ...", and one
the agent starts with `run`, "Schedule “{name}”, run by the agent at {time}".

## Spike

None.

## Prior art

* Codex reuses a conversation's thread for its automations, so one wakes
  the work it belongs to
  ([Codex for (almost) everything][codex]).
* ChatGPT names its one-time, recurring and change-watching tasks
  Scheduled ([Scheduled tasks in ChatGPT][chatgpt]); Claude Code renamed
  its routines scheduled tasks ([Claude Code docs][claude]).

[codex]: https://openai.com/index/codex-for-almost-everything/
[chatgpt]: https://help.openai.com/en/articles/10291617-scheduled-tasks-in-chatgpt
[claude]: https://code.claude.com/docs/en/routines

## Alternatives

* **Keep the account's page and add the window.** Two places show the
  same thing, and the page still offers what no conversation needs.

## Open questions

None.
