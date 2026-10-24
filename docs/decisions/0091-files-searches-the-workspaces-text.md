---
status: proposed
---

# Files searches the text of the workspace's files

## Problem

A user wants to open the file that defines `retryDelay` in a project their
agent is working on, or the one named `retry-policy.ts` somewhere in it.
Files lists directories and completes a file's name in its address bar, one
folder at a time, but it cannot find a file by its name across the tree or
look inside files. The user has to ask the agent,
which spends a turn and tokens on a grep, or open a terminal on the Host.

## Decision

A magnifier in the tree's caption, beside Refresh, floats a field over
Files, as the conversations' search floats over the app. Typing searches the
names and the text of the files under the workspace on the conversation's
Host and lists each file found. Return opens the selected file, at the line
its text was found at.

```
┌ Files ─────────────────────────────────────┐
│   ┌────────────────────────────────────┐   │
│   │ 🔍 retry                           │   │
│   ├────────────────────────────────────┤   │
│   │ ▌retry-policy.ts  src/net          │   │
│   │  client.ts  src/net                │   │
│   │    export const retryDelay = 250   │   │
│   │  network.md  docs                  │   │
│   │    A request that fails waits `ret…│   │
│   └────────────────────────────────────┘   │
└────────────────────────────────────────────┘
```

What the user sees:

* The tree's caption has a magnifier before Refresh. It, or Search in Files,
  ⇧⌘F (Ctrl+Shift+F), a shortcut of the keyboard settings, floats the field
  at the top of Files, focused. Escape, or a click outside, closes it.
* Find, ⌘F (Ctrl+F), another shortcut of the keyboard settings, finds where
  the user last pressed or moved the focus, as VS Code's ⌘F finds in the
  part that has the focus. In Files it floats this field. In a file's text
  or a diff, the editor's own find bar takes the keys first. In another view
  of the work panel the keys are left to the browser. Anywhere else it opens
  the conversations' search, which so replaces the browser's find in the
  page there.
* Typing searches once it pauses, 200 ms as the conversations' search does,
  for the text as it is, letter case aside. There is no pattern, no toggle
  of case or words, and no glob: one field.
* A file whose name holds the text lists first, the text marked in the
  name. A file whose text holds it lists after, with the first line that
  does under its name, the text marked there; a long line shows the part
  around its first match. Each kind is in the tree's order, as the files
  come in.
* ↑ and ↓ move the selection, Return or a click opens the file, at that
  line for a line, marked as `FileContent` marks a line it opens at, and the
  field closes.
* The field is as tall as its list, up to a limit past which the list
  scrolls. A search that reaches its limit says so under the list.

What is searched:

* Every file under the workspace root the tree shows, hidden files included,
  as VS Code does.
* Files the workspace's `.gitignore`, `.ignore` and git's exclude file leave
  out are skipped, as VS Code's Quick Open skips them. The `.git` directory
  is always skipped, and symbolic links are not followed.
* A file's name is searched whatever the file holds. Its text is searched
  only if Files shows it as text: up to `EDIT_FILE_BYTES` (8 MiB), and not
  binary by `is_binary`, no NUL byte in its first 8 KiB and no media type it
  recognizes. A file that is not UTF-8 is searched as Files shows it, each
  byte that is not UTF-8 a U+FFFD.
* A file's text is read to its first line that holds the text.
* The search stops at 5,000 files, as Zed's does.

### Runner

The runner searches on the Host with ripgrep's libraries
(`crates/runner-host/src/search.rs`): `ignore` walks the directory on up to
eight threads and applies the ignore files, `grep-regex` builds a matcher of
the text, escaped and without regard to case, and `grep-searcher` finds a
file's first line that holds it.

```rust
// crates/runner-protocol/src/search.rs
pub struct SearchQuery {
    /// Found as it is, letter case aside; one line.
    pub text: String,
}

/// One line of the output, as JSON.
#[serde(tag = "type")]
pub enum SearchLine {
    /// A file found: its path relative to the root, with `/`, where its
    /// name holds the text, and the first line of its text that does.
    File { path: String, name: Vec<TextRange>, line: Option<FoundLine> },
    /// The last line.
    End { files: u32, limit_reached: bool },
}

pub struct FoundLine {
    /// 1-based.
    pub number: u32,
    /// The line, or the part of a long line around its first match, with
    /// `…` where it was cut.
    pub text: String,
    /// In UTF-16 code units of `text`, as the page slices it.
    pub matches: Vec<TextRange>,
}

// crates/runner-protocol/src/wire/messages.rs
FsSearch { id: String, root: String, query: SearchQuery, output: PipeRef }
```

* The reply, `FsResult::Search`, comes once the root is a directory. The
  output pipe then carries the lines as the walk finds the files, in no
  order, and the page sorts them.
* Output that ends without `End` was cut short. An empty line, after 15
  seconds without another, says the search still runs, so nothing between
  the page and the Host closes a quiet connection.
* When the output closes, the walk stops: more typing, which starts another
  search, or the field closing ends the one before it.

### Between the page and the runner

* Relay: `POST /api/conversations/{id}/fs/search` with `{ root, query }`,
  answered as the runner's lines (`application/x-ndjson`), streamed and
  leased as a download is.
* Direct: a channel op `search` with the same fields, answered with `Opened`
  and then the lines as bytes. Closing the channel stops the search, also
  before the runner answered.
* The wire version rises to 63: an older runner is refused and updates
  itself.

## Spike

1. **Speed without an index.** A prototype with `ignore`, `grep-regex` and
   `grep-searcher` searched a repository of 3,854 files (ignore files
   applied) in 31–61 ms. The first match arrived in 3–19 ms. A second
   repository of 3,994 files took 39–59 ms. With the ignore files off, the
   same tree held 291,896 files (37 GB, build output included) and took
   4.6–5.5 s. The first match still arrived within 60 ms, and the toggle is
   off by default.
2. **Size of the runner.** The runner already links `regex`, `aho-corasick`,
   `memchr`, `bstr` and `walkdir`. A pair of binaries built with the
   project's release profile differed by 0.7 MB with the new libraries. The
   runner is about 40 MB.

## Prior art

* Zed
  ([`project_search.rs`](https://github.com/zed-industries/zed/blob/main/crates/project/src/project_search.rs),
  [`search.rs`](https://github.com/zed-industries/zed/blob/main/crates/project/src/search.rs))
  keeps no index of file contents. It walks its in-memory file list,
  filters paths first, then parallel workers read each file's first 8 KB to
  skip binaries and look for one match in 64 KB blocks. Only a file that
  matches is loaded whole and searched for every match. It stops at 5,000
  files or 10,000 matches. Search on type with a 250 ms pause is a setting.
  For a remote project the host runs the search and the client receives
  only the matching files.
* Helix
  ([`global_search`](https://github.com/helix-editor/helix/blob/master/helix-term/src/commands.rs))
  uses the same ripgrep libraries this design uses: a parallel `ignore` walk,
  `grep-searcher` that stops at a NUL byte, and results pushed to the list
  as found, until the list is closed.
* VS Code
  ([`ripgrepTextSearchEngine.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/search/node/ripgrepTextSearchEngine.ts),
  [`match.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/search/browser/searchTreeModel/match.ts))
  runs the ripgrep program with `--json --hidden`, honors ignore files unless
  told not to, and stops at 20,000 results, cutting the line that reaches
  the limit. It lists each match as a row with up to 26 characters before
  it and 250 in all.

Where this departs:

* VS Code and Zed keep finding files by name (Quick Open, ⌘P) apart from
  searching their text, which has toggles for case, words and patterns, and
  globs of files. Demi has one field for both, as the conversations' search
  finds a conversation by its title or its messages, and no options.
* Zed detects a file's encoding and decodes it, so a Latin-1 `café` is
  found and shown as such. Demi searches what Files shows, which decodes
  only UTF-8.
* Zed and VS Code list every match. Demi lists each file once, with its
  first line that matches, since what the user does next is open it.
* Zed sorts results in the runner. Demi lets the walk run in parallel and
  sorts on the page.

## Alternatives

* **An index of the files' contents**, as JetBrains keeps one. It makes a
  search faster on very large trees, but it has to be built, stored on every
  Host and kept current with each change. A walk is fast enough for a
  project's tree.
* **Running the `rg` program.** It is not on every Host, and shipping it
  with the runner adds a second binary for each platform. The libraries add
  0.7 MB to the runner.
* **A panel where the tree is, with Enter to search and options for case,
  words, patterns and globs**, as VS Code's Search view. It was built and
  set aside: in a tree's width its results showed little of each line, its
  options crowded the field, and finding a file to open needs none of them.
* **Search on Enter.** Each pause in typing walks the Host, but the walk
  before it stops, and a project's tree takes tens of milliseconds.

## Open questions

None.
