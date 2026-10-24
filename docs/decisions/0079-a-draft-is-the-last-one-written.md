---
status: proposed
---

# A draft is the last one written, and nothing comes back as Not Saved

## Problem

Two things appear above and under the composer that the user did not ask
for and cannot place:

* A notice above the composer, `A later save replaced "…"`, with Restore.
  It shows when a page whose copy of the draft was out of date saved it:
  the backend kept the text it replaced and offers it back on every page,
  until the user restores or dismisses it.
* A row in the Saved list, `… Not saved`, holding a message the user
  already sent. A Save for Later, or an edit of a saved message, that the
  backend refused stays there, and a page reload can replay an edit the
  backend had already taken, which then finds its message sent and is
  refused.

No messaging app keeps a replaced draft to offer back, or lists a message
the user sent as not saved.

## Decision

**A conversation's draft is the last one written, on whichever page.**
Every page shows it as it changes; a page that has input the backend has
not yet taken keeps its own until that input lands. No version a later
save replaced is kept, and no notice offers one back.

**A message saved for later, or an edit of one, is on its way until the
backend takes it,** shown as Saving and sent again after a reload; once
taken it leaves the browser's queue. When the backend refuses it:

* A Save for Later's text goes back into the composer, and a toast says
  why.
* An edit of a message that was sent or deleted meanwhile is dropped, and
  a toast says that the message was sent or deleted.

The list shows only messages the backend keeps, and those on their way.

The design to build from:

* `PUT /conversations/:id/draft` takes no `base`; the backend writes it
  over the draft. `DraftSave.base`, `ConversationDraft.replaced`,
  `ReplacedDraft`, `ReplacedDraftAction`, the replaced draft's route and
  `DraftChanged` are removed, and a migration drops `conversation_drafts`'s
  `written`, `replaced_revision` and `replaced`; a replaced version still
  held at the upgrade is dropped with its files, as a later save replaced it.
* The page keeps the draft's `revision` to tell a newer draft from its
  own. A composer the backend has not accepted keeps, on the page only and
  never sent, the revision its text was written over; a restored one whose
  revision the backend's draft has passed gives way to that draft.
* The browser's stored draft keeps the queue of saves and edits on their
  way (`savedWrites`), a Save for Later included, and drops `notSaved`; its
  schema version goes up, and the upgrade puts each Not Saved message back
  in the composer's text and files.
* The Saved list's states are Saved and Saving; `ReplacedDraftNotice` and
  the Not Saved row, with its Retry, are removed from `web-ui` and the
  gallery.

## Spike

None.

## Prior art

* Telegram's cloud draft is the last one set, with no replaced version;
  a client does not apply an update of the draft while its user is
  changing it
  ([TDLib updateChatDraftMessage](https://core.telegram.org/tdlib/docs/classtd_1_1td__api_1_1update_chat_draft_message.html)).

## Alternatives

* **Keep the notice, shown only on the page whose save replaced the
  other.** It still asks the user about a version they did not write on
  that page.

## Open questions

None.
