---
status: proposed
---

# Files searches the text of the workspace's files

## Problem

A user wants to find where `retryDelay` is used in a project their agent is
working on. Files lists directories and completes a file's name in its
address bar, but it cannot look inside files. The user has to ask the agent,
which spends a turn and tokens on a grep, or open a terminal on the Host.

## Decision

The tree's place in Files switches to Search. A query searches the text of
every file under the workspace on the conversation's Host and lists each
match, grouped by file. A click on a match opens the file at its line. Search runs when the user presses Enter, not while they type. There is
no replace.

```
┌ Files │ Search ───────────────────┐
│ [retryDelay          ] Aa ab .*   │
│ ▸ files to include / exclude      │
│ 14 results in 5 files             │
│ ▾ src/net/client.ts            3  │
│     12  const retryDelay = 250    │
│     40  await sleep(retryDelay)   │
│     41  retryDelay *= 2           │
│ ▾ dist/app.min.js              2  │
│     1   var c={retryDelay:250};…  │
│     1   …t,e.retryDelay*2**0))}…  │
│ ▸ docs/network.md              1  │
└───────────────────────────────────┘
```

What the user sees:

* The tree's place gets two tabs, Files and Search. Search in Files, ⇧⌘F
  (Ctrl+Shift+F), a shortcut of the keyboard settings, opens the shown
  conversation's Files at Search with the query selected.
* Three toggles: Match Case, Match Whole Word, and Use Regular Expression. A
  folded row holds globs of files to include and to exclude, separated by
  commas, and a box that also searches the files `.gitignore` leaves out. A
  folder's name takes everything in it, as VS Code's fields do.
* Results appear as files are found, each file in its place in the tree's
  order, with its count. Each match is a row, as VS Code lists them: its
  line's number and the part of the line around it, the match marked. A
  line with three matches is three rows, so every match is in sight and
  counted, a match far into a long line included.
* A summary says how many results in how many files, "Searching…" while it
  runs, and when the limit stopped it. A pattern that is not one says why
  under the field, in the Host's words.
* Enter again, or a new query, stops the search that runs and starts over.
  The results stay while the user switches back to Files and returns. They
  go when the view does, which stops a search that runs, or when the
  conversation's root or Host changes.
* A click on a match opens the file at its line, marked as `FileContent`
  marks a line it opens at. Down from the field moves into the results, where
  the arrow keys move and fold and Enter opens.

What is searched:

* Every file under the workspace root the tree shows, hidden files included,
  as VS Code does.
* Files the workspace's `.gitignore`, `.ignore` and git's exclude file leave
  out are skipped unless the toggle is on. The `.git` directory is always
  skipped.
* The globs to include and to exclude choose among the files the ignore
  files leave in, as Zed's filters do: including `*.log` never brings back a
  log `.gitignore` leaves out. ripgrep's `--glob`, which VS Code passes the
  fields to, overrides the ignore files, and brings back an ignored file but
  not one in an ignored folder.
* Symbolic links are not followed.
* A file is searched only if Files shows it as text: up to
  `EDIT_FILE_BYTES` (8 MiB), and not binary by `is_binary`, no NUL byte in
  its first 8 KiB and no media type it recognizes. Other files are skipped.
* A file that is not UTF-8 is searched as Files shows it, each byte that is
  not UTF-8 a U+FFFD, so a pattern matches what its result shows: `caf. cr`
  finds a Latin-1 `café crème`.
* The search stops at 10,000 matches or 5,000 files, as Zed's does. A file
  stops at the matches that still fit, so the count is exact and a file of
  millions of matches costs no more than the limit.

### Runner

The runner searches on the Host with ripgrep's libraries
(`crates/runner-host/src/search.rs`): `ignore` walks the directory on up to
eight threads and applies the ignore files, `grep-regex` builds the matcher (a
plain query is escaped, so one matcher serves both kinds), and
`grep-searcher` finds the matching lines of each file.

```rust
// crates/runner-protocol/src/search.rs
pub struct SearchQuery {
    pub pattern: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Globs relative to the root; none means every file.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_ignored: bool,
}

/// One line of the output, as JSON.
#[serde(tag = "type")]
pub enum SearchLine {
    /// A file that matched: its path relative to the root, with `/`.
    File { path: String, matches: Vec<TextMatch> },
    /// The last line.
    End { files: u32, matches: u32, limit_reached: bool },
}

/// One match, one result.
pub struct TextMatch {
    /// The line's number, 1-based.
    pub line: u32,
    /// The line, or the part of a long line around the match, with `…`
    /// where it was cut.
    pub text: String,
    /// The match in `text`, in UTF-16 code units, as the page slices it.
    pub start: u32,
    pub end: u32,
}

// crates/runner-protocol/src/wire/messages.rs
FsSearch { id: String, root: String, query: SearchQuery, output: PipeRef }
```

* The reply, `FsResult::Search`, comes once the query is read and the root
  is a directory. The output pipe then carries the lines as the walk finds
  the files, in no order, and the page sorts them.
* Output that ends without `End` was cut short. An empty line, after 15
  seconds without another, says the search still runs, so nothing between
  the page and the Host closes a quiet connection.
* A pattern or a glob that is not one answers the code `invalid_pattern`
  with the regex library's words.
* When the output closes, the walk stops, between files and between the
  lines of one: a page that starts another search, or goes away, ends the
  one before it.

### Between the page and the runner

* Relay: `POST /api/conversations/{id}/fs/search` with `{ root, query }`,
  answered as the runner's lines (`application/x-ndjson`), streamed and
  leased as a download is. A bad pattern answers 400 `invalid_pattern`.
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

* Zed detects a file's encoding and decodes it, so a Latin-1 `café` is
  found and shown as such. Demi searches what Files shows, which decodes
  only UTF-8.
* Zed shows each matching line whole in an editor of excerpts. Demi's
  results are a list, as VS Code's are, so each match is a row with the
  part of its line around it.
* Zed reads a matching file whole and collects every match in it, and
  stops only between files. Demi stops a file at the matches that fit.
* ripgrep, and so VS Code, report a match of a pattern of bytes, such as
  `(?-u:.)`, that falls inside a character. Zed refuses such a pattern.
  Demi widens the match to whole characters.
* Zed sorts results in the runner. Demi lets the walk run in parallel and
  sorts on the page, since a search starts only on Enter and its results are
  few.

## Alternatives

* **An index of the files' contents**, as JetBrains keeps one. It makes a
  search faster on very large trees, but it has to be built, stored on every
  Host and kept current with each change. A walk is fast enough for a
  project's tree.
* **Running the `rg` program.** It is not on every Host, and shipping it
  with the runner adds a second binary for each platform. The libraries add
  0.7 MB to the runner.
* **Search on type.** It starts a walk of the Host for each pause in typing,
  over a relay for some Hosts. The user chose Enter.

## Open questions

None.
