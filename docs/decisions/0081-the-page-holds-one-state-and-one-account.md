---
status: proposed
---

# The page holds one state, whose parts may be missing, and one account, which a sign-out forgets

## Problem

The browser keeps a copy of the account on disk, so that a page opened
offline, or reopened, shows the sidebar and a conversation at once. Three
things about it are wrong.

* **A copy stands in for the state.** The copy is the whole product state,
  25 parts, and a page that finds it stops its read of `GET /state` and
  follows the backend from the copy's version. A list of Settings that a
  view asked for before the copy came waits for a read of the state that
  never comes. Alex opens `/settings/users` in a new tab and sees "Loading
  users…" until he reloads; it happened in 2 of 16 openings of Settings
  sections, whichever came first deciding it.
* **The copy keeps what no offline view shows.** The sign-in methods, the
  passkeys, the signed-in sessions, the install commands and the lists of
  Settings are written to disk with the sidebar's list.
* **An account's data is in several places, and a sign-out clears one.**
  The copy is deleted at sign-out. The drafts, the folded projects, a
  draft's last words, the devices' measured paths and the plugins' cache of
  a Host's files stay; the plugins' cache is not even the account's, so the
  next account that signs in may first see the files the last one opened.

## Decision

Nothing changes on screen while the page is online: the sidebar and the
conversation show at once from the copy, as today, and the rest as it is
read. Offline, the sidebar and the conversations the copy holds show; the
rest keeps loading until the backend answers. A Settings section opened by
its address always loads. After a sign-out, nothing of the account is left
in the browser.

### One state, whose parts may be missing

The page holds one product state (`useProduct`). Any part of it may be
missing, which means not known yet; a view of a part that is missing shows
it loading, by the part's own status, as the lists of Settings do today.

```ts
/** The state the page holds: each part is missing until it is known. */
type PageState = Partial<ProductState> & HeldLists

/** Where a part stands: there, still to come, or not coming. */
function partLoad(part: keyof PageState): 'ready' | 'loading' | 'failed'
```

* The copy, when the page finds it, fills the parts it holds and nothing
  else. It holds no version.
* Every start reads `GET /state`, which replaces the state whole, and the
  channel follows the backend from the version that read names. A list a
  view holds is read once the state is read, whether a copy showed or not.
* `snapshot` stays the one state the views read; nothing else holds a part
  of it. The checks of whether "the state" is there become checks of the
  part a view shows.

### The copy holds what shows the conversations

| Part | What reads it before the state is read |
| --- | --- |
| `conversations` | The sidebar's list, a conversation's record |
| `workspaces` | The sidebar's projects, the header's Project |
| `devices` | The header's Host, the projects' Hosts |
| `plugins` | The sidebar's entries of plugins, such as Skills |
| `preferences` | The language, the theme, the text size, hidden models, the shortcuts |
| `publicUrl` | The header's menu and a reply's Share |
| `providers`, `models` | The composer's model button |
| `subagents` | The composer's Workflow button |
| `user` | The account row, and the sign-in when the backend cannot be reached |

The conversations' records, their nodes and their blocks stay as they are.
Nothing else of the state is written to disk.

### One account, which a sign-out forgets

One module, `state/account-data.ts`, owns what the browser keeps of the
signed-in account and is the only one that names it:

| Kept | Where |
| --- | --- |
| The copy, the conversations, their nodes and blocks | IndexedDB `demi-offline` |
| The drafts | IndexedDB `demi-drafts`, by account |
| The plugins' cache of a Host's files | IndexedDB `demi-plugin-cache` |
| The folded projects, a draft's last words | `localStorage`, `demi.preferences.<account>`, `demi.draft.<account>.*` |
| The devices' measured paths | `localStorage`, `demi.deviceMeasured.*` |
| Which account the data is | `localStorage`, `demi.account` |

* The browser holds one account's data at a time. A sign-in of another
  account than `demi.account` names forgets the one before first.
* A sign-out forgets the account: the user's Sign Out, and a session that
  ended, which an answer of 401 or the channel's `session_ended` says.
  Every store above is emptied, then `demi.account` goes.
* A draft not yet saved to the backend, one written while offline, goes
  with the account.
* What belongs to the browser stays: the theme, the language, the
  notification setting, the catalog of providers to add.
* A write that comes after the account was forgotten writes nothing, as the
  copy's writes do today.

### What an older page or data meets

The copy is written by one build and read by that build alone, so an older
copy is dropped, as today. The drafts and the local keys keep their shape.
The plugins' cache, kept by no account before, is emptied once at the
first start of this build.

## Spike

None ran for this record. The parts of the copy are the ones a spike of
the record this one replaces measured: with them, a page whose read of the
state was blocked, and one whose read was delayed by 1.5 seconds, showed
the sidebar, the header, the transcript and the composer pixel for pixel as
with the whole copy. The acceptance measures it again on the running
product.

The cost: every start reads the state, where a reload with a current copy
read nothing before.

## Prior art

None followed. Demi's own lists of Settings already hold the rule this
record gives every part: missing until read, loading meanwhile.

## Alternatives

* **Keep the whole copy and read the held lists after it shows.** It fixes
  the Settings sections and keeps on disk what no offline view shows.
* **A second state of the parts the copy holds, beside the state read**
  (the record this one replaces). Two places then hold the conversations
  and the devices, and each view has to choose which one it reads.
* **One database per account.** The browser never holds two accounts at
  once, so a name per account adds nothing that forgetting at sign-out does
  not give, and each store would move.

## Open questions

None.
