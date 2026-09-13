# Edit tracking

A shell job reports the files it created or modified, each with its diff, so
the conversation can show what one tool call edited. The runner learns this
from the commands themselves as they run, not from the filesystem afterwards:
every in-process file write passes through a layer the runner owns, and that
layer takes notes. This is why the shell runs in process and why the utilities
are forked to open files through one context.

Edit tracking has one consumer: the file pills under a shell call in the
conversation, and the work panel's change view in Conversation mode that a
pill opens to show what that call changed. Nothing else reads it. The model
never receives it; the working tree view ([Runner](runner.md#working-tree))
answers a different question, "what is uncommitted?", for the whole directory.
The two share the file entry shape and nothing else.

## Scope

| Recorded | Not recorded |
| --- | --- |
| A file a brush redirection opens for writing (`>`, `>>`, `<>`, `exec 3>f`). | Anything an external child process does: git, python, node, user-installed tools. |
| A file an embedded utility opens for writing, creates, writes whole, copies to, or renames over (`sed -i`, `tee`, `cp`, `sort -o`). | Deletions, renames as moves, directories, permissions, ownership, times. |
| Every in-process part of the job: subshells, functions, background tasks, process substitutions. | Reads, and the empty file `mktemp` creates. |

An entry is `added` when the path did not exist at the job's first write to it,
`modified` otherwise. A path whose bytes are the same at exit as before the
first write is not reported: `touch`, a write of identical content, a temporary
file removed before exit.

The limits keep tracking cheap and bounded whatever a script does:

| Limit | Value | Beyond it |
| --- | --- | --- |
| File size, before or after | 8 MiB | The entry keeps its path and kind; counts are 0 and 0 and it has no diff. Binary content is treated the same. |
| Diff size, per file | 256 KiB | The entry keeps its counts and has no diff. |
| Diff size, per job | 1 MiB | Entries past the total keep their counts and have no diff. |
| Recorded paths per job | 500 | Later writes go unrecorded and the report says `filesTruncated`. |

## How a job records

```text
sed -i 's/a/b/' src/main.rs                 # inside job J, cwd /work

utility ──open for write──▶ context::fs ──write notice──▶ Scope J
                                                          │ first notice for this path:
                                                          │   copy current bytes → J/changes/0
                                                          │   (or note: did not exist)
                                                          ▼
                                                     open proceeds, sed writes, renames
job exits ─────────────────────────────────────────▶ Scope J
                                                      for each recorded path:
                                                        read current bytes, diff against J/changes/0
                                                        → kind, added, removed, unified diff
                                                      delete J/changes
                                                      job_exit.files = [{path: "src/main.rs", kind: "modified", diff: "@@ ...", ...}]
```

One job owns one `Scope`, the object the runner hands to brush as its execution
host and to the utilities as their execution control. Both ask the scope to
resolve paths against the invocation's cwd; both now also send it a write
notice with the resolved path before any open that can change a file's content:
an open with write, append, truncate, create, or create-new set; a whole-file
write; a copy, a rename, or a hard link, for their destination. A utility that
writes through a temporary file and renames it over the target notifies for the
target, so `sed -i` reports `src/main.rs`, not its temporary name. Read-only
opens send nothing.

On the first notice for a path the scope copies the file's current bytes into
the job's directory, or notes that the path does not exist, before the open
proceeds. Later notices for the same path are counted and nothing more. The
order matters: a truncating open that ran first would leave nothing to copy.

When the job exits, however it ended, the scope reads each recorded path's
current bytes, diffs them against the copy line by line with the same algorithm
the working tree uses, renders the hunks as a unified diff with three lines of
context, and reports the entries in `job_exit`. The copies are deleted with the
report; nothing about an edit stays on the target. Recording never fails the
command: a copy that cannot be taken, a path that disappeared, or a diff that
cannot run leaves that entry with counts 0 and 0 and no diff, and the command's
own result stands.

## The report

`job_exit` carries `files`, one entry per changed path in the order of first
write, and `filesTruncated`:

| Field | Meaning |
| --- | --- |
| `path` | Relative to the job's starting directory when under it; absolute otherwise. |
| `kind` | `added` or `modified`. |
| `added`, `removed` | Lines added and removed; 0 and 0 when there is no diff. |
| `diff` | The hunks as a unified diff, without the file header. Absent under the limits above or for binary content. |

The entry shape is the working tree's ([Runner](runner.md#working-tree))
without `deleted`, `renamed`, and `from`, plus `diff`.

## Delivery to the conversation

The host records `files` on the command's exit status, and the shell tool puts
them in its `ShellToolView.files` for the rendering layer
([Tool rendering](../tool-rendering-spec.md)). The view is stored with the
block, so a call's edits remain readable, diffs included, after the runner, the
job, the target, or the backend is gone, and reading them never touches the
host.

The shell block shows the entries as file pills under the call. Picking a pill
opens the work panel's change view in Conversation mode on that call, with the
picked file selected: the tree lists that call's files, and the selected file
shows its hunks. An entry without a diff shows why (binary, or over the
limits). Conversation mode never lists anything on its own and never refreshes:
its content is one call's report, and picking a pill of another call replaces
it.

## Crates and packages to change

| Where | Change |
| --- | --- |
| `vendor/brush-core` | The execution host trait gains the write notice; the shell's file opening sends it for every write-capable option set. |
| `vendor/uucore` | The execution control trait gains the write notice; `context::fs` sends it from creating and write-capable opens, whole-file writes, and the destination of copy, rename, and hard link. A helper persists a temporary file over a target with the notice. |
| `vendor/sed` | In-place editing persists through that helper. |
| `crates/runner` | `Scope` implements both notices, keeps the copies in the job's directory, diffs at exit, and reports; the line diff moves out of the working tree module to be shared; `job_exit` gains the fields. |
| `packages/runner-protocol` | `job_exit` schema and the generated Rust bindings. |
| `packages/shell`, `packages/host-remote` | The command's exit status carries `files`. |
| `packages/agent` | `ShellFileChange` gains `diff`; the shell tool view carries `files`. |
| `packages/web-ui` | The pill opens the change view; Conversation mode renders hunks from a report instead of reading sides. |
| `packages/web`, `packages/web-gallery` | Product wiring and specimens. |

The other vendored utilities need no change: `cp`, `mv`, `tee`, `sort`,
`touch`, and `uniq` already open through `context::fs`; `sort` and `tac` use
anonymous temporary files; `ripgrep`, `jaq`, `findutils`, and `diffutils` do
not write files.

## Rationale

Watching the filesystem or diffing the working tree around a call cannot say
which call made a change, misses edits outside the repository, and blurs
concurrent calls. Notices from the write path are exact for everything that
runs in process and free of scanning. External processes are outside that path
by nature; their edits still appear in the working tree view, unattributed.

The report carries the diff rather than references to copies on the target
because the conversation is read long after the job: on another device, after
a Cloud reset, after the runner is replaced. A diff of bounded size travels with
the block; copies on the target would need a lifetime, a route, and a host that
is up.

Only creations and modifications are reported because the conversation shows
what the model wrote, not the state of the directory. A move is a deletion the
view would not show and a creation it does show; reporting the destination as
`added` says what the reader needs.
