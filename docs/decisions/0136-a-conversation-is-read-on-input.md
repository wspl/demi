---
status: proposed
---

# A shown conversation is read once the user acts on the page, and the tab marks what is unread

## Problem

Alex leaves Demi on the conversation *Draft the release notes* and works in
another app. Its turn finishes: the sidebar's dot turns green, the browser
tab's title says `(1 done)` and its icon wears a dot. Alex comes back to
the browser, and the moment the page is visible again every mark is gone:
the tab forgets its count as the page comes forward, and the sidebar's dot
clears too, because the page marks the conversation it shows as read
whenever it becomes visible or focused. Nothing says the conversation
finished, and Alex has to work out that it was the one already on screen.
The tab and the sidebar follow two rules besides: the tab counts only what
happened since the page was last in front, and leaves out the open
conversation, while the sidebar's dots stay until each is read.

The same happens when the window stays visible behind another app: the
page reads each new revision as it arrives, so the marks never show at
all.

## Decision

The conversation a page shows is read when the user **acts on the page**:
a click or a tap, a key, or a wheel scroll, while the page is visible and
focused. Becoming visible or focused is not acting, and neither is the
pointer moving over the page, which it does on the way back to it.

* While the user acts on the page, it stays **attended**: what arrives in
  the shown conversation is read at once, as today, so a user watching a
  turn finish never sees it marked.
* The page stops being attended when it is hidden or loses the focus. What
  arrives from then on stays unread until the first action after the user
  comes back.
* The tab follows the sidebar's dots with one rule: its title counts, its
  icon's dot shows and an installed app's badge numbers every conversation
  whose dot says something is new, the open one included, until it is
  read. Coming forward no longer clears the tab.
* Opening a conversation, from the sidebar, a link or a notification, is an
  action: it is read as it shows its transcript, as today.

```text
  focused, acting     | away (hidden or blurred) | back      | first click
  --------------------+--------------------------+-----------+------------>
  arrivals read       | arrivals stay unread:    | dot and   | read: dot
  at once             | dot, tab title, badge    | tab stay  | and tab clear
```

### Design

`packages/web` owns one value, whether the page is attended:

```ts
/** True from the user's first action on the visible, focused page until it is hidden or blurred. */
const attended: Ref<boolean>
```

* `pointerdown`, `touchstart`, `keydown` and `wheel` on the window, in the
  capture phase, and the same input a frame of the page's own passes on
  (`FRAME_INPUT`, which now carries the input's type), set it while
  `document.visibilityState === 'visible'` and `document.hasFocus()`.
  `visibilitychange` to hidden clears it, and so does the window's `blur`
  unless the focus went into a frame of the page. A focus lost from within
  such a frame tells the window nothing, so the read asks `attending()`,
  which clears `attended` once the page is no longer in front.
* `markRead` acts only while `attending()`, in place of its check of
  visibility alone. The watch in `App.vue` that marks the shown
  conversation runs on `attended` too, so the first action reads what
  arrived while the user was away. The `focus` and `visibilitychange`
  handlers in `main.ts` no longer mark it.
* A change of the conversation the page shows sets `attended` while the
  page is in front, since only the user changes it: a click in the sidebar,
  a link, a shortcut or Back. A page that loads in a window the user is not
  at, as one reloading for a new build does, is not attended.
* A notification's click acts on another surface, for the conversation
  already shown as well. Its worker tells the page before it brings the
  page forward, so the page is attended as it next comes in front.

* `followTabAttention` counts the news of the rows the sidebar lists, as
  `conversationDotTone` reads them, and nothing else: it no longer tracks
  when the page was in front, which conversation is open, or which page
  came forward last to own the badge, since every page counts the same.
  A page of a signed-in account that shows no sidebar, such as a handoff
  code's, knows nothing of the news and leaves the badge as the account's
  other pages set it.

Nothing changes on the backend or in what is stored.

## Prior art

* Element Web (`apps/web/src/UserActivity.ts`) sends read receipts only
  while the user is active: a mouse press or move, a key or a wheel while
  the document has the focus, and counts the window's `focus` itself as
  activity. Its timeline keeps a read marker line where the unread part
  began.
* Slack marks the channel in view read when its window regains the focus
  and keeps a *New* divider in the channel.

Demi has no line in the transcript where the unread part begins, so the
sidebar's dot and the tab are what tell the user where to look. Like
Gmail's tab, which counts the unread mail whatever was read in another
tab, the tab counts what is unread rather than what happened while away.
This departs from both by not counting the focus, or the pointer's move,
as an action.

## Alternatives

* **Read on focus, with a "new" line in the transcript**, as Element and
  Slack do. The sidebar's dot still vanishes before the user sees it,
  which is the problem.
* **Keep the marks for a few seconds after coming back.** A fixed delay
  says nothing about whether the user saw it.

## Open questions

None.
