---
status: accepted
---

# A wait always shows its spinner

Supersedes, in part: 0038 (a pane changes whole), where a pane that has waited
300 ms says what it reads in words alone.

## Context and Problem Statement

The user opens the Changes tab on a far Host. The area under the address is
empty, then says "Reading Changes" in plain grey text, with no spinner. Every
other loading region in Demi, a list, a file, a dialog, a menu, draws a
spinner above its sentence, so this one looks like a note or a failure, not
like something that is still going on.

0038 replaced the spinner a pane showed while it read with an empty area and
a sentence after 300 ms, and wrote the sentence as text only. 0049 then said it
from the start of a panel's first view, which made the missing spinner what
the user sees at every page load.

## Considered Options

* Keep the words alone, as 0038 drew them.
* Draw the words as every loading region does: a spinner above the sentence.

## Decision Outcome

Chosen option: a pane's reading sentence is the loading region (`RegionStatus`
with `status="loading"`), because one wait should look the same wherever it
is. 0038's timing is unchanged: a view that a switch brings says it only past
300 ms, a panel's first view from its start.

The menu row that stands for a list still being read, "Reading Workflows…", shows
the spinner too, as a menu row whose operation runs does.

```text
  Changes tab, its reads in flight

  before:  [                    ]      after:  [                    ]
           [   Reading Changes  ]              [         ◠          ]
           [                    ]              [   Reading Changes  ]
```

### Consequences

* Good, because every wait, in a pane, a region or a menu, reads as waiting.
* Neutral, because the 300 ms delay of 0038 still keeps a fast read from
  flashing the spinner.
