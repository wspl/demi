---
status: accepted
---

# A selection opens a comment field

Supersedes, in part: 0020 (a selection is resolved once), where a selection
shows a toolbar of the object's tools.

## Summary

Selecting words in a reply opens a field to type a remark at once, with
Comment, Ask and Send under it, in place of a toolbar of Comment and Ask that
each opened a field of its own. The right-click menu's Comment and Ask, and a
comment on an image, open the same field. One field replaces the three that
did the same thing.

## Motivation

A user reads a reply, selects "rebuilt on every start" and wants to say it
should not be. Today the selection shows a toolbar; a click on Comment opens a
field below the text, and the words go there: two steps for one remark.
Quoting the words into a message takes a copy and a paste. The right-click
menu's Comment opens a second kind of field, left-aligned under the text with
no close button, and its Ask a third; each says "Write first" differently and
takes different keys.

## Guide-level explanation

On the desktop, the field opens above the selection, centered on it, and
takes the focus, so the first key, an input method's included, types into
it:

```text
                    ┌───────────────────────────────────────────┐
                    │ |Add a comment                          ✕ │
                    │                   Send   Ask   [Comment]  │
                    └───────────────────────────────────────────┘
  …the cache is rebuilt on every start, which costs about 2 s…
```

* The selected words stay drawn as selected, Cmd-C copies them while the
  field has nothing selected of its own, and a right-click on them opens the
  menu as before.
* Return comments, the default; Cmd-Return sends.
* Comment: the words become a comment on the selection, which goes with the
  next message.
* Ask: the words become a side question about the selection, answered in a
  card in the reply. A page that takes no side questions shows no Ask.
* Send: the selection, quoted, and the words go to the main agent now.
* With no words, Return, Comment and Ask shake the field once; Send does
  nothing and is shown dimmed.
* ✕, Escape, a click elsewhere or a new selection closes the field and drops
  what was typed in it.
* While the reply is still being written, the buttons are disabled and say to
  wait.

On a phone the field opens in a bottom sheet, the app's own `Sheet`, without
a scrim: the page above it stays as it was and takes touches, so the
selection's handles still extend it, and the field acts on the selection as
it is when an action is chosen. The sheet stands clear of the system's menu
by the selection, and the keyboard comes when the field is tapped. A swipe
down on the sheet, or a tap on the page that clears the selection, closes
the field and drops what was typed.

The sheet's first line quotes what the field acts on, in one line cut with
an ellipsis, as a comment quotes its words: the selected text as it is now,
with each image or other media in it said as `[Media]`, and an image's
comment said as `[Media]` alone. The desktop field, which stands by the
selection, quotes nothing.

```text
  ┌──────────────────────────────┐
  │ …the cache is [rebuilt on    │
  │ every start], which costs…   │
  │                              │
  │╭────────────────────────────╮│
  ││ ▏rebuilt on every start    ││
  ││ |Add a comment             ││
  ││          Send  Ask [Comment]││
  │╰────────────────────────────╯│
  └──────────────────────────────┘
```

## Reference-level explanation

All in `web-ui`; no route or frame changes.

```ts
/** What the field does with the words typed in it; an absent action has no button. */
interface SelectionFieldActions {
  /** Comments on the selection; the words are never empty. The field keeps them until it settles. */
  comment: (words: string) => void | Promise<void>
  /** Asks a side question about the selection; the words are never empty. */
  ask?: (words: string) => void | Promise<void>
  /** Sends the quote and the words to the main agent now; the words are never empty. */
  send?: (words: string) => void | Promise<void>
  /** Why the field cannot act now, such as a reply still being written. */
  readonly unavailable?: SentenceText | null
}

/** What the field shows: the selection read and its object's actions. */
interface SelectionFieldOffer extends ResolvedSelection {
  actions: SelectionFieldActions
}

interface ContextMenuDeclaration {
  /** The field's actions for this object's own text selected; none for no field. */
  selectionField?: (selected: ResolvedSelection) => SelectionFieldActions | null
}

interface ReplySelectionTools {
  /** The field's actions for a selection in a reply, or null while it takes none. */
  field: (reply: { blockId: string; root: HTMLElement }, selected: ResolvedSelection, streaming: () => boolean) => SelectionFieldActions | null
}
```

* **One field.** `SelectionField` is the only field for a remark on a part of
  a reply. The right-click menu's Comment and Ask, and Comment on an image,
  open it anchored to the selection or the image; the earlier comment field
  is removed, and the send field stays only as the follow-up field of an
  answered side question.
* **Focus.** On the desktop the field takes the focus when it opens,
  without scrolling: a browser starts an input method's composition only in
  a focused editable element, so a key that reaches the page first arrives as
  a plain letter. The page's selection is drawn by the comment highlight
  while the field has the focus; the field's `copy` writes the selected
  words when the field has no selection of its own; a right-click on the
  words restores the page's selection for the menu and gives the focus back
  when the menu closes.
* **Placement.** On a pointer device, centered above the selection's rect,
  flipped below when there is no room above, kept inside the view. On a touch
  device, in a `Sheet` without a scrim, which leaves the page interactive;
  the same component, not a second sheet.
* **Closing.** On the desktop, a press outside the field and the selection
  closes it. On a phone, a swipe down on the sheet or the selection cleared
  closes it.
* **Phone.** The phone's selection shows only the field, for objects that
  declare `selectionField`. Send to Conversation, which the phone's toolbar
  offered for every selection, is no longer on a phone; the desktop's
  right-click menu keeps it.

## Upgrade and version skew

None: the change is in the page alone.

## Performance

No change: the field replaces the toolbar, and nothing is loaded or asked for
until an action.

## Drawbacks

* The field covers more of the reply than the toolbar did.
* While a field is open, every key goes into it: Space no longer scrolls,
  and the user's own single-key shortcuts do not reach the page.
* The selection cannot be extended from the keyboard while the field is
  open.
* A phone can no longer send a selection to another conversation, and a
  selection in a user's own message, a shared page, an archived conversation
  or a subagent's transcript shows nothing on a phone.

## Rationale and alternatives

* Keep the toolbar: two steps for the commonest act.
* Show the field only on a click, as a toolbar's Comment does: one step
  fewer, still a click before the words.
* Pin the field by the selection on a phone too: iOS places its menu above
  or below the selection as it fits, and it covered the field's line.
* Pin the field to the screen's bottom without a sheet: it stands over the
  message field, two fields in one place.
* A sheet with a scrim: the scrim covers the selection's handles, and a word
  selected by a long press could not be extended.
* A Comment button first, the sheet on its tap: one more tap each time.

## Prior art

* Google Docs and Notion show a button or a toolbar by a selection, and their
  comment field opens on a click or a shortcut (Cmd-Option-M); Medium shows a
  toolbar of Highlight, Respond and Share.
* ChatGPT shows "Ask ChatGPT" by a selection, which quotes it into the
  message field.
* On phones, Google Docs opens a comment in a sheet from the bottom of the
  screen; Apple Maps keeps its sheet open without a scrim while the map
  above it takes touches.
* Demi departs in opening the field at once and taking the first key into
  it; no mainstream product does this, which the user chose for the
  commonest act.

## Unresolved questions

None.
