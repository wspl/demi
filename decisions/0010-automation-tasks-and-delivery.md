---
status: accepted
---

# Automation tasks in a repeating cycle, delivery to a busy conversation, and runs that manage automations

## Context and Problem Statement

Three limits of automations stand in the way of how people use them:

* One automation has one schedule and one message. Work planned over a week,
  such as "every Monday at 10:00 sum up last week's failed payments" and
  "every Wednesday at 20:00 sort this week's to-dos", needs two automations,
  each with its own target, model and switch; and every two weeks, every
  month, every quarter or every year can be said only in cron, or not at
  all.
* A message to an existing conversation that is busy always waits in the
  queue until the turn ends. A user who wants it to reach the running turn,
  as Steer does in the composer, cannot ask for that.
* An agent working on an automation's run may not create, change, delete or
  run any automation, and may show only its own. A run therefore cannot
  adjust its own automation, such as moving its next time or turning itself
  off once its job is done.

## Considered Options

* A repeating automation has one cycle, such as every week, and holds
  several tasks placed in it, each a moment in the cycle with its own
  message; its target, model, switch and delivery are shared. A run may
  manage automations like any agent, with the user's Manage Automations.
* One automation per schedule, grouped in the list by a shared name.
* Several tasks, each with a repeat rule of its own.

## Decision Outcome

Chosen option: one cycle with tasks placed in it, because work that is
planned over a week or a month, as "Monday morning this, Wednesday evening
that", is set up the way it is thought of, and what the tasks share, where
runs go, with which model, and whether they run at all, is set once.

### Repeat and tasks

* A repeating automation has a Repeat: a cycle of days, weeks, months or
  years, every one or every several of them, as Calendar's repeat works.
  The menu offers Every Day, Every Week, Every 2 Weeks, Every Month, Every
  3 Months, Every Year and Custom, where Custom sets any count of any unit.
  So every two weeks, every two months, every quarter and every half year
  are all a Repeat.
* A cycle longer than its unit starts on a date the automation keeps, its
  first cycle; every 3 months from January runs in January, April, July
  and October.
  Nothing runs before that date, even in the first cycle's unit: every 2
  months from January 20 with a task on the 1st first runs on March 1.
* When the backend was down at times a task should have run, each task runs
  once for its latest missed time, as a single trigger does today.
* The automation holds one or more tasks. A task is a moment in the cycle
  and a message:

  | Repeat | A task's moment |
  |---|---|
  | Days | A time |
  | Weeks | One or more weekdays and a time, so every weekday at 09:00 is one task |
  | Months | A day of the month, the last day, the first to fourth or last of a weekday, or the first or last working day (Monday to Friday), and a time |
  | Years | A month, a day as for months, and a time |

  In a cycle of several units, every task runs in the cycle's first unit,
  such as the first week of every two.
* A day a month lacks, such as the 31st, is skipped in that month, as
  Calendar does; the last day runs every month, as a payday does.
* A task on the first or last working day may name a country, and a region
  where its holidays differ. A working day is then a day of that country's
  working week that is not one of its public holidays; with no country,
  Monday to Friday.
  Weekend days a government declares working days, such as China's make-up
  workdays, are not counted.
* The holidays are a table of dates exported once from the `holidays`
  Python package (MIT, its notice copied with them) into the repository,
  for every country and region it covers, through 2077, the end of the
  package's own Islamic calendar tables. Porting the package's rules
  instead was refused: about 65,000 lines of Python whose lunar and Islamic
  dates are themselves tables ending in 2077 or 2100, so they would reach
  no further than the export does. Nothing fetches them at
  run time. A holiday announced after the copy, such as a one-off day off,
  and a year's official schedule that differs from the package's estimate
  are missing until the table is copied again.

* Once, every so many minutes or hours, cron, when output changes and when
  a URL is called stay one trigger with one message.
* Shared by all tasks: the name, the switch, the time zone, the target (this
  conversation, an existing one, or a new one each run with its device and
  project), the model, and the delivery below.
* The automation's next run is the earliest next run of its tasks. Each run
  records which task started it. An automation set today to run daily on
  every day becomes Every Day with one task, and one set to some days only
  becomes Every Week with one task on those days.

### The editor

Each choice is one menu, as Calendar's and Reminders' are, and no row
carries an explanation under it.

```text
 Weekly Ops                                       [Run Now ▾]  [On]
 Next run Mon 10:00.

 When           Every Week ▾
 Tasks          Mon 10:00   Sum up last week's failed payments…
                Wed 20:00   Sort this week's to-dos…
                + Add Task
 Time zone      Asia/Shanghai ▾
 Conversation   New Each Run ▾
 Device         Cloud ▾
 Model          Claude Sonnet ▾
```

* When is one menu: Once, Every Day, Every Week, Every 2 Weeks, Every Month,
  Every 3 Months, Every Year; then Every N Minutes or Hours…, Cron…, When
  Output Changes and When a URL Is Called; then Custom…, whose sheet sets
  the count, the unit and the first cycle's date. A single trigger shows
  its own fields, such as the command or the URL, and its message, in place
  of the tasks.
* A task's row opens its sheet: its day as the unit asks, its time and its
  message. A task on a working day also shows Holidays there: None, or a
  country and its region. The row's ··· menu holds Run Now and Delete.
* Conversation is one menu: New Each Run; then the recent conversations;
  then Choose…. Device and Project follow for New Each Run; If Busy (Queue,
  Steer) follows for an existing conversation.

```text
 ┌ Last Working Day 17:00 ──────────────────────────────┐
 │ Day        Last Working Day ▾                         │
 │ Holidays   China ▾                                    │
 │ Time       17:00                                      │
 │ Message    Send this month's payroll summary…         │
 │                                              [Done]   │
 └───────────────────────────────────────────────────────┘
```

* Run Now in the header runs the only task, or offers the tasks when there
  are several.
* Add Task adds a task at the cycle's first moment at 09:00 with an empty
  message, and opens it; an empty message is shown as missing, and that task
  does not run until it has one.

### Delivery to a busy conversation

* An automation that wakes an existing conversation, this one or another,
  chooses what its message does when that conversation is busy: Queue, the
  default and today's behavior, waits until the turn ends; Steer reaches
  the running turn, as the composer's Steer does.
* A new conversation each run is never busy, so the setting does not show
  for it.

### Runs manage automations

* An agent working on an automation's run lists, shows, creates, changes,
  runs and deletes automations as any agent does, its own included.
* What guards it is the same as anywhere: changing automations needs the
  user's Manage Automations in that conversation.

### Consequences

* Good, because work planned over a cycle lives in one automation, any
  common cycle is a menu choice, and a run can manage its own automation.
* Good, because a message can reach a running turn when the user wants it to.
* Bad, because a run started by another system's call can now make or change
  automations once its conversation holds Manage Automations.
* Modules touched: the automations' protocol, storage (with the migration of
  today's automations to one task each) and scheduler, `demi automation`'s
  commands, the delivery of a run's message, and the Automations page in the
  gallery and the product.
