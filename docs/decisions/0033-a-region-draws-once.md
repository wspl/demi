---
status: accepted
---

# A region draws once

## Context and Problem Statement

Some parts of a reply or a file draw in two steps, because the code that
draws them loads after the first paint: code turns from plain to
highlighted, a formula from its TeX source to the equation, a file icon from
a plain outline to its own, an editor from empty to its text. A formula's
height changes when it does; the others change only how they look.

## Considered Options

* Draw the plain form first, as now.
* Load what a region needs before it draws, and keep its size.

## Decision Outcome

Chosen option: load first, because the plain form shows for a moment only
and draws the eye to the change.

* Once the first conversation has shown, the page loads in the background
  the highlighter with its common languages, the math renderer and its
  fonts, the editor, and the file icons' list. A file icon's own picture
  loads when one is first needed; its box has its size from the start, so
  nothing moves when it arrives.
* A region is a code block or run of code wherever it shows (a reply, a
  message, a command's row, a file), a formula, a file icon or an editor.
  One that is shown before all it needs is in, its renderer, its language,
  its fonts or its picture, waits to draw, for at most 300 ms in all, at the
  size it will have where that is known; past that it draws its plain form,
  and it changes once, when everything is in. Its frame and its label, such
  as a code block's language, show at once.
* What waits is the region and what follows it that has not shown yet.
  Nothing already on screen is hidden again: a formula that streams into a
  paragraph leaves the paragraph's words where they are, and a plain form
  once shown stays until the final one replaces it.
* A formula's plain form is its TeX source; an editor's, its text without
  highlighting.
* The background loading runs on every page that shows a conversation: the
  app, a shared conversation and the gallery. It never competes with what a
  shown region is waiting for.

```text
  Created both files as requested.            shown
  ┌───────────────────────────────────────┐
  │ TypeScript                          ⧉ │   frame and label shown
  │                                       │   code waits, ≤ 300 ms
  └───────────────────────────────────────┘
                                              the formula, and what
                                              follows it, wait with it
```

### Performance

Measured on a production build:

| Loaded in the background | Transferred | Parsed script |
|---|---|---|
| Highlighter with common languages | 187 KB | 534 KB |
| Math renderer and its fonts | 341 KB | 257 KB |
| Editor (beyond what is loaded) | 26 KB | 77 KB |
| File icons' list | small | small |

* The first conversation does not wait for any of it: loading starts after
  it shows.
* Risk, measured: preloading every file icon's picture as well (1,251
  modules, 927 KB) would delay the first conversation by 0.13 s at 300 ms
  and 0.56 s on a slow 4G link and take a minute to finish, so icons are not
  preloaded.
* About 0.55 MB more is transferred on a cold start, which a slow 4G link
  takes about 3 s to load in the background.

### Consequences

* Good, because a reply's code, formulas and icons show in their final form.
* Bad, because the page loads those parts before it needs them.
