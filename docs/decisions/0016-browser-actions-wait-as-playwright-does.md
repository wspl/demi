---
status: accepted
---

# Browser actions wait as Playwright does

## Context and Problem Statement

A `demi browser click` on a button that only changes its own text takes
about 300 ms, while Playwright clicks the same button in about 33 ms. Most of
the difference is one wait: after the input, the click waits a fixed 250 ms
to see whether a navigation starts. The rest is work done for every action
whatever it needs: the whole page's structure is read before a reference or a
CSS selector is resolved and again at cleanup, and the mouse's move, press
and release each wait for the browser's answer before the next is sent.

An agent that drives a page step by step pays this at every click, and a flow
of many steps reads as slow even when it runs as one script.

## Considered Options

* Align each action's waits and steps with Playwright's.
* Shorten the fixed wait.
* Keep the actions as they are.

## Decision Outcome

Chosen option: align with Playwright, because its behavior is the one agents
and their models already know, and it gives the same guarantees without
waiting on a clock.

* After the input, an action returns as soon as the browser has handled it.
  When the input started a navigation, which the browser signals as it starts,
  the action waits for that navigation as it does today. A navigation that a
  page starts later, from a timer, is the next command's to wait for, with
  `demi browser wait` or `--wait-url`, as in Playwright.
* An action reads only what it needs: a reference or a CSS selector is
  resolved directly, and cleanup releases what the action used without
  reading the page again. A role or a name still reads what the match needs.
* Input events go out in order without waiting for each answer, as
  Playwright sends them, while an answer that fails still fails the action.
* The checks before an action, that the element is visible, stable, enabled
  and receives the click, follow Playwright's.
* The browser's help says that a sequence of steps is one script: commands
  chained in one call, each result read with `--json` to decide the next.

### Consequences

* Good, because a click on a page that does not navigate takes tens of
  milliseconds instead of about 300, and a scripted flow runs at the speed of
  the page.
* Good, because an action waits for exactly what Playwright waits for, so an
  agent's expectations from Playwright hold.
* Bad, because a page that navigates from a timer between 100 and 250 ms
  after a click is no longer awaited by the click; the agent waits for it
  explicitly, as it already must for one later than 250 ms.
