---
status: proposed
---

# A page in the background opens the update by itself

Supersedes: 0123 (An update ends on Open Demi), for a page that was not in
front when the update began.

## Problem

Alex starts an update from Settings › Updates in one tab, and has two more
tabs of Demi open behind it, and Demi on his phone, locked. Every page shows
the update screen, and every page ends on **Open Demi**: after watching the
update in the first tab, Alex finds the same finished screen in each other
tab and on his phone, and opens Demi in each by hand. Those pages showed the
screen to no one; the button only adds a click for each.

## Decision

The update screen ends on **Open Demi** only in a page that was in front
when the update began: a tab the user was looking at, in a window that was
not minimized. A page that was in the background then loads the new version
by itself once the update ends, so the user who comes back to it finds Demi
updated.

```text
the update begins     the page is            when the update ends
──────────────────    ────────────────────   ──────────────────────────────
in front              shows the screen       waits on Open Demi, as 0123
in the background     shows the screen       loads the new version itself
```

* A page counts as in front when the browser says it is visible
  (`document.visibilityState`, the Page Visibility API): another tab in
  front, a minimized window and a locked phone are the background.
* The moment that counts is the update's start, when the page first learns
  of it. A page brought to the front during the update still loads by
  itself at its end: its user did not watch the update begin, and the
  screen they see then is the same one they would have opened past.
* An update that returned to the release it left loads that release in the
  same way. Settings › Updates says the update failed, as today.
* While the update runs, every page shows the screen, in front or not, and
  the app under it takes no input, as today.

The design to build from: the page records whether it was visible when it
noted the update (`packages/web/src/state/update.ts`), and once the update
ends, a page that was not loads itself, as **Open Demi** would.

## Spike

None.

## Prior art

* [Home Assistant](https://github.com/home-assistant/frontend/blob/dev/src/layouts/ha-init-page.ts)
  reloads its page by itself once its upgrade is done, as 0123 notes; Demi
  does so only where no one watched the update begin.
* The [Page Visibility API](https://developer.mozilla.org/docs/Web/API/Page_Visibility_API)
  is the browser's own answer to whether a page is seen: a hidden tab, a
  minimized window and a locked phone are all `hidden`.

## Alternatives

* **Decide when the update ends, by whether the page is in front then**: a
  user who switches to a tab in the last second of an update would find it
  waiting, though they watched none of it, and one who switches away a
  second before its end would miss its result.
* **Load every page by itself, as before 0123**: the page the user watches
  would leave before showing every step done.

## Open questions

None.
