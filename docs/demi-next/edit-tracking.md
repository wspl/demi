# Edit tracking

A shell job reports the files it created or modified, so the conversation can
show what one tool call edited: the list under the call, and each file's diff
on request. The runner learns this from the commands themselves as they run,
not from the filesystem afterwards: every in-process file write passes through
a layer the runner owns, and that layer takes notes. This is why the shell runs
in process and why the utilities are forked to open files through one context.
Native Demi file commands also participate in recording, as described below.

Edit tracking has one consumer: the file pills under a shell call in the
conversation, and the work panel's change view in Conversation mode that a
pill opens to show what that call changed. Nothing else reads it. The model
never receives it; the working tree view ([Runner](runner.md#working-tree))
answers a different question, "what is uncommitted?", for the whole directory.
The two share the file entry shape and the diff view and nothing else.

The list travels with the transcript block. The file contents behind the diff
do not: they live in the conversation's change store, described below, and the
browser fetches them when a file is opened.

## Scope

| Recorded | Not recorded |
| --- | --- |
| A file a brush redirection opens for writing (`>`, `>>`, `<>`, `exec 3>f`). | Writes by external programs that do not participate in recording: git, python, node, user-installed tools. |
| A file an embedded utility opens for writing, writes whole, or renames over (`sed -i`, `tee`, `sort -o`, `uniq` with an output file). | Copies and hard links (`cp`), deletions, renames as moves, directories, permissions, ownership, times. |
| Files created or modified by `demi file create`, `demi file edit`, and `demi file patch`, including when one patch edits several files. | Edits prepared by a native command but never written, or successfully rolled back. |
| Every in-process part of the job: subshells, functions, background tasks, process substitutions. | Reads, and the empty file `mktemp` creates. |

An entry is `added` when the path did not exist at the job's first write to it,
`modified` otherwise. A path whose bytes are the same at exit as before the
first write is not reported: `touch`, a write of identical content. A path that
does not exist at exit is not reported either, whether it existed before the
job or was created during it.

| Limit | Value | Beyond it |
| --- | --- | --- |
| File size, before or after | 8 MiB | The entry keeps its path and kind; counts are 0 and 0 and it has no contents. Binary content is treated the same. |
| Bytes copied per job | 64 MiB | Later paths are recorded with their kind only. |
| Recorded paths per job | 500 | Later writes go unrecorded and the report says `filesTruncated`. |

## How a job records

```text
sed -i 's/a/b/' src/main.rs                 # inside job J

utility ──open for write──▶ context::fs ──write notice──▶ Scope J
                                                          │ first notice for this path:
                                                          │   copy current bytes → J/changes/0.original
                                                          │   (or note: did not exist)
                                                          ▼
                                                     open proceeds, sed writes, renames
job exits ─────────────────────────────────────────▶ Scope J
                                                      for each recorded path:
                                                        copy current bytes → J/changes/0.modified
                                                        count lines added and removed
                                                      job_exit.files = [{path, kind, added, removed,
                                                                         original: "J/changes/0.original",
                                                                         modified: "J/changes/0.modified"}]
```

One job owns one `Scope`, the object the runner hands to brush as its execution
host and to the utilities as their execution control. The notice is the only
point that knows a write is happening; the scope and everything after it work
on paths alone. Both brush and the utilities ask the scope to resolve paths
against the invocation's cwd; both also send it a write notice with the
resolved path before any open that can change a file's content:
an open with write, append, truncate, create, or create-new set; a whole-file
write; a rename, for its destination. A utility that writes through a temporary
file and renames it over the target notifies for the target, so `sed -i`
reports `src/main.rs`, not its temporary name. Read-only opens send nothing.

On the first notice for a path the scope copies the file's current bytes into
the job's directory, or notes that the path does not exist, before the open
proceeds. Later notices for the same path are counted and nothing more. The
order matters: a truncating open that ran first would leave nothing to copy.

When the job exits, however it ended, the scope copies each recorded path that
still exists beside the first copy, counts the lines added and removed between
the two with the algorithm the working tree uses, and reports the entries in
`job_exit`. The copies stay in the job's directory with its retained output,
under that directory's lifetime ([Pipes and output](runner.md#pipes-and-output)).
Recording never fails the command: a copy that cannot be taken for a remaining
path leaves that entry with counts 0 and 0 and no contents, and the command's
own result stands.

## Native Demi file edits

Native Demi file commands run in a separate command service
([Native execution](native-runtime.md#one-command-from-declaration-to-result)).
Their edits are part of the invoking shell job's report. The command service must supply
recording information through its invocation protocol, since its filesystem
calls do not pass through brush or `context::fs`. The runner remains responsible
for the job's report, limits, and retained copies.

Recording describes actual file changes. For example, if a patch writes the
first file, fails on the second, and restores the first, it must not report the
prepared patch as an applied edit. If restoration fails, recording must reflect
the changes left behind. The scope rules above still exclude deleted files.

## Open recording decisions

The first-write/job-exit snapshots described above do not isolate edits made by
concurrent jobs. If A writes `1`, B overwrites it with `2`, and A then exits,
A's final copy contains `2`. Capturing after each edit can avoid that late read,
but one job can also edit the same file more than once with another job's edit
in between. One comparison of its first and last copies can still include the
other job's changes.

Before implementing dependent recording behavior, settle when one edit begins
and ends, how its before/after contents are captured during concurrent writes,
and how interleaved edits appear under one call. Define the native command
protocol exchange against that same recording contract. These decisions are
unresolved; the native commands' inclusion in scope is required.

## The report

`job_exit` carries `files`, one entry per changed path in the order of first
write, and `filesTruncated`:

| Field | Meaning |
| --- | --- |
| `path` | Absolute, as the target names it. A persistent shell keeps its cwd across jobs, so the job's directory is not the workspace root; the browser shows the path relative to the workspace root when under it, absolute otherwise, with `/` separators. |
| `kind` | `added` or `modified`. |
| `added`, `removed` | Lines added and removed; 0 and 0 when there are no contents. |
| `original`, `modified` | The two copies on the target, as paths; `original` is absent for an added file. Both absent when the contents were not kept. |

The entry shape is the working tree's ([Runner](runner.md#working-tree))
without `deleted`, `renamed`, and `from`.

## The change store

When a command's exit reaches the backend, the backend reads each entry's
copies from the target and writes them into the conversation's change store,
then hands the tool its status with the list. The read is part of completing
the command inside the agent's own host session, the same way the command's
retained output is read back; it is not an operation on the host from outside
the agent. A copy that cannot be read or stored leaves its entry in the list
without contents; the list is never lost over the contents.

The change store is one namespace of the object store that also holds blobs
([Storage](storage.md#attachment-and-transcript-media)): a directory locally,
S3 in deployment. Its objects are bound to the conversation, not addressed by
content:

```text
changes/<conversationId>/<commandId>/<n>.original
changes/<conversationId>/<commandId>/<n>.modified
```

`n` is the entry's index in the command's list. The objects belong to the
conversation's owner and live as long as the conversation; an archived
conversation keeps them. They are written before the block that lists them is
checkpointed, under the blob rule that a committed block never points at
unpublished bytes.

The transcript block's `ShellToolView.files` carries the list only:
`path`, `kind`, `added`, `removed`, and `kept`, true when both contents are in
the store.

`GET /api/conversations/:id/commands/:commandId/changes/file?path=...` returns
`{ original, modified }` for one entry from the store, `original` empty for an
added file; 404 `not_found` when the command has no such entry or its contents
were not kept. No host is involved.

## Delivery to the conversation

The shell block shows the entries as file pills under the call, from the block's
view ([Tool rendering](../tool-rendering-spec.md)). Picking a pill opens the
work panel's change view in Conversation mode on that call, with the picked
file selected: the tree lists that call's files, and the selected file's two
sides, fetched through the route above, show in the same diff editor the
Uncommitted mode uses. An entry that is not `kept` remains in the list but has
no diff; the interface silently omits the diff without a message or explanation.
Conversation mode never lists anything on its own and never refreshes:
its content is one call's list, and picking a pill of another call replaces it.

## Crates and packages to change

| Where | Change |
| --- | --- |
| `vendor/brush-core` | The execution host trait gains the write notice; the shell's file opening sends it for every write-capable option set. |
| `vendor/uucore` | The execution control trait gains the write notice; `context::fs` sends it from creating and write-capable opens, whole-file writes, and the destination of a rename. A helper persists a temporary file over a target with the notice. |
| `vendor/sed` | In-place editing persists through that helper. |
| `crates/demi-commands` | Record actual edits from file create, edit, and patch, including failure and rollback outcomes. |
| `packages/command-protocol`, `crates/command-service` | Carry native edit-recording information within the invoking command; the exchange depends on the open recording decisions above. |
| `crates/runner` | `Scope` implements both notices, keeps the copies in the job's directory, counts lines at exit, and reports; the line counting moves out of the working tree module to be shared; `job_exit` gains the fields. |
| `packages/runner-protocol` | `job_exit` schema and the generated Rust bindings. |
| `packages/shell`, `packages/host-remote` | The command's exit status carries the entries with their copy paths. |
| `packages/backend` | The change store; the shell environment wrapper that moves copies into it and hands the tool the list; the route. |
| `packages/agent` | The shell tool view carries the list with `kept`. |
| `packages/web-ui` | The pill opens the change view on its call; Conversation mode is one call's list with a `read` that the host supplies. |
| `packages/web`, `packages/web-gallery` | The product's `read` through the route; specimens. |

The other vendored utilities need no change: `tee`, `sort`, `touch`, and
`uniq` open through `context::fs`; `sort` and `tac` use anonymous temporary
files; `ripgrep`, `jaq`, `findutils`, and `diffutils` do not write files.

## Rationale

Watching the filesystem or diffing the working tree around a call cannot say
which call made a change, misses edits outside the repository, and blurs
concurrent calls. Notices from the write path identify which job attempts a
write without scanning the directory. Native Demi commands participate explicitly
through their invocation protocol. Other external programs remain outside that
path; their edits still appear in the working tree view, unattributed.

The list is in the block because it is small and is read every time the
conversation is shown. The contents are not, because a transcript is loaded
whole and a file is up to 8 MiB: they are read only when a file is opened, so
they live in a store that is fetched by entry. They are bound to the
conversation rather than content-addressed because they are the conversation's
history and go with it, and because two sides of one edit are only meaningful
together.

Only creations and modifications are reported because the conversation shows
what the model wrote, not the state of the directory.
