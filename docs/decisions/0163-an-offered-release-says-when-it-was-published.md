---
status: accepted
---

# An offered release says when it was published, and its background download is a bar

## Problem

Alex runs a Demi server on the Canary channel. Settings › Updates says
"Demi 0.1.5500 is available", and Alex cannot tell whether that build came
out an hour ago, and may still be followed by a fix, or a week ago, and has
been run by everyone since. The page does not say.

While the server downloads the release ahead of Update Now, the second line
reads "Downloading in the background: 88 MB of 214 MB". The number is
there, but how far along the download is takes reading two sizes and
dividing them, and the sentence is cut off on a phone, where the line also
holds Release Notes.

## Decision

Settings › Updates, while it offers a newer release:

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Demi 0.1.5500 is available  3 days ago                [Update Now]   │
│ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━──────────  88 MB of 214 MB  │
│ Release Notes ↗                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

* **When it was published.** Beside the version, in the row's secondary
  color, the time since GitHub published the release, in the words the page
  uses for "Last checked": "3 days ago", which moves on as time passes. On a
  phone, where the first line has no room for it, it goes before Release
  Notes: "3 days ago · Release Notes".
* **The background download is a bar.** The second line is a progress bar
  that fills the room the text leaves, followed by "88 MB of 214 MB". The
  sizes keep the room of the widest they can read, the total's or the
  "1,023 KB" just before a unit changes, so the bar does not change its
  length as the digits grow. Assistive technology
  still reads "Downloading in the background". The moments without a size
  to show stay words: the start of a download, before the server knows its
  size, and Installing in the background, of which the server reports no
  progress.
* **Release Notes** moves to a third line of its own, on a computer and on
  a phone, so the second line keeps the whole width for the bar or the
  state. The line is there in every state, so the row keeps its height as
  one replaces another: it links the notes of the newer release a check
  found, and otherwise, while the server is up to date or checks, the notes
  of the release it runs.

The sidebar's "Demi 0.1.5500 is available" stays as it is.

### Design

The newest release GitHub answers carries `published_at`, which the check
reads today and drops. A check that found a newer release reports it as one
value, since its version, its notes and its date exist only together:

```rust
pub enum UpgradeCheck {
    Never,
    Checking,
    Checked { at: Timestamp, newer: Option<OfferedRelease> },
    Failed { at: Timestamp, reason: String },
}

pub struct OfferedRelease {
    pub version: String,
    /// The page of its release, with its notes.
    pub notes_url: String,
    /// When GitHub published it.
    pub published_at: Timestamp,
}
```

`ServerUpgrade` also carries `running_notes_url`, the page of the running
release, which `releases::page_url` gives as it gives the offered one's.

`shared-artifacts`'s `releases::newest` returns the release's version and
its publication time; Canary takes the time of the highest tag's release
from the list it already reads. The backend keeps the check in memory, so
no stored data changes, and the web app comes with the backend that serves
it, so no older page reads the new shape.

## Spike

None.

## Prior art

* The App Store on iOS and macOS lists each update with the time since it
  came out ("3 Days Ago") beside the app's name, and its notes under it.
* macOS's Software Update and Safari's downloads show a download as a bar
  with "88 MB of 214 MB" beside it.

## Alternatives

* **The date in the second line** ("Released 3 days ago · Ready to
  install"): the line already changes with the download, and on a phone the
  date leaves no room for the state.
* **The third line only while a release is offered, held blank otherwise**:
  the row keeps its height, but an up-to-date server shows an empty line
  where the notes of the release it runs are of use.
* **Release Notes at the right end of the first line**: tighter, but it puts
  a link between the version and Update Now that reads as part of either.

## Open questions

None.
