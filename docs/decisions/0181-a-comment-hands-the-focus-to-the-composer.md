---
status: proposed
---

# A comment hands the focus to the composer

## Problem

A user selects a sentence in a reply, types "Use the staging URL here" in the
comment field and presses Return. The comment joins the composer's waiting
comments and the field closes, but the focus goes nowhere: to send, the user
reaches for the mouse and clicks the composer, or Send, first.

## Decision

Once a comment is saved and its field has closed, the composer takes the
focus, its caret at the end of the draft, so the next Return sends the draft
with its comments, and typing adds words to it. This holds for a comment on
a picture too. Ask and Send leave the focus as they do today: Ask opens its
card in the reply, and Send has already sent.

While the composer shows an edit of an earlier message, a new comment still
waits in the draft, but the focus stays where it is: Return in the edit would
send the edit without the comment.

The comment field calls what its object says follows a comment once it has
closed on it, after it clears the page's selection, so the composer's caret
is the last selection:

```ts
interface SelectionFieldActions {
  comment: (words: string) => void | Promise<void>
  /** What follows a comment once the field has closed on it. */
  commented?: () => void
  // ask, send and unavailable as before
}

interface ComposerFocus {
  /** Puts the focus in the composer, its caret at the draft's end. */
  focus(): void
}
```

The conversation's surface (`ChatSession`) provides a `ComposerFocus`
(`provideComposerFocus`); its composer (`SessionComposer`) takes it while
mounted (`takeComposerFocus`), and the transcript's comments (`ReplyComments`)
give `commented` as its `focus`. A field closed before its comment was saved,
as Escape closes it, calls nothing. A phone has no comment field, so nothing
changes there.

## Spike

None.

## Prior art

Demi's composer already takes the focus when it is handed the next step: New
on the draft already shown, and a message taken back into the composer. A
comment saved to the draft is the same case, as its next step is sending.

## Alternatives

- Leave the focus on the page: the user must click before Return sends.
- Send the draft at once on Return in the comment field: Send in the field
  already sends one comment alone; several comments are often gathered
  before one message.

## Open questions

None.
