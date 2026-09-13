# Edit tracking

A shell job reports the files it created or modified, with the line counts
and both sides of each, so the conversation can show what one tool call
edited. The runner learns this from the commands themselves as they run, not
from the filesystem afterwards: every in-process file write passes through a
layer the runner owns, and that layer takes notes. This is why the shell runs in
process and why the utilities are forked to open files through one context.

Edit tracking answers "what did this call edit?" for one tool call. The working
tree view ([Runner](runner.md#working-tree)) answers "what is uncommitted?" for
the whole directory. They share the file entry shape and nothing else.

## Scope

| Recorded | Not recorded |
| --- | --- |
| A file a brush redirection opens for writing (`>`, `>>`, `<>`, `exec 3>f`). | Anything an external child process does: git, python, node, user-installed tools. |
| A file an embedded utility opens for writing, creates, writes whole, copies to, or renames over (`sed -i`, `tee`, `cp`, `sort -o`). | Deletions, renames as moves, directories, permissions, ownership, times. |
| Every in-process part of the job: subshells, functions, background tasks, process substitutions. | Reads. |

An entry is `added` when the path did not exist at the job's first write to it,
`modified` otherwise. A path whose bytes are the same at exit as before the
first write is not reported: `touch`, a write of identical content, a temporary
file removed before exit.

The limits keep tracking cheap and bounded whatever a script does:

| Limit | Value | Beyond it |
| --- | --- | --- |
| Snapshot size, either side | 8 MiB | The entry keeps its path and kind; counts are 0 and 0 and it has no sides. Binary content is treated the same. |
| Recorded paths per job | 500 | Later writes go unrecorded and the report says `filesTruncated`. |

## How a job records

```text
sed -i 's/a/b/' src/main.rs                 # inside job J, cwd /work

utility ──open for write──▶ context::fs ──write notice──▶ Scope J
                                                          │ first notice for this path:
                                                          │   copy current bytes → J/changes/0.original
                                                          │   (or note: did not exist)
                                                          ▼
                                                     open proceeds, sed writes, renames
job exits ─────────────────────────────────────────▶ Scope J
                                                      for each recorded path:
                                                        copy current bytes → J/changes/0.modified
                                                        diff original/modified → added, removed
                                                      job_exit.files = [{path: "src/main.rs", kind: "modified", ...}]
```

One job owns one `Scope`, the object the runner hands to brush as its execution
host and to the utilities as their execution control. Both ask the scope to
resolve paths against the invocation's cwd; both now also send it a write
notice with the resolved path before any open that can change a file's content:
an open with write, append, truncate, create, or create-new set; a whole-file
write; a copy or a rename, for their destination. A utility that writes through
a temporary file and renames it over the target notifies for the target, so
`sed -i` reports `src/main.rs`, not its temporary name. Read-only opens send
nothing.

On the first notice for a path the scope copies the file's current bytes, or
notes that it does not exist, before the open proceeds. Later notices for the
same path are counted and nothing more. The order matters: a truncating open
that ran first would leave nothing to copy.

When the job exits, however it ended, the scope copies each recorded path's
current bytes, diffs the two sides line by line with the same algorithm the
working tree uses, and reports the entries in `job_exit`. Recording never fails
the command: a snapshot that cannot be taken, a path that disappeared, or a
diff that cannot run leaves that entry with counts 0 and 0 and no sides, and
the command's own result stands.

Snapshots live in the job's directory beside its retained output, with that
directory's lifetime ([Pipes and output](runner.md#pipes-and-output)); they are
not copied to the backend.

## The report

`job_exit` carries `files`, one entry per changed path in the order of first
write, and `filesTruncated`:

| Field | Meaning |
| --- | --- |
| `path` | Relative to the job's starting directory when under it; absolute otherwise. |
| `kind` | `added` or `modified`. |
| `added`, `removed` | Lines added and removed between the sides; 0 and 0 when there are no sides. |
| `sides` | The snapshots on the target, `{ original?, modified }` as paths; `original` is absent for an added file. Absent when a side could not be kept. |

The entry shape is the working tree's ([Runner](runner.md#working-tree))
without `deleted`, `renamed`, and `from`, plus `sides`.

## Delivery to the conversation

The host records `files` on the command's exit status, and the shell tool puts
them in its `ShellToolView.files` for the rendering layer
([Tool rendering](../tool-rendering-spec.md)). The view is stored with the
block, so the files a call edited remain visible after the runner, the job, or
the backend is gone; only the sides depend on the target still holding them.

The shell block shows the entries as file pills under the call. Picking a pill
opens the work panel's change view in Conversation mode on that call: the tree
lists that call's files, and a selected file shows its diff. The view reads a
side through the device file text route
([Web API](web-api.md#file-text-and-working-tree-changes)), the route the file
view already uses, with the snapshot's path; a side the target no longer holds,
or one over the route's text limit, shows the view's unavailable text. The
Conversation mode never lists anything on its own and never refreshes: its
content is one call's report.

## Open decision

The model does not see `files` today: the view carries them and the tool's
output text does not. Whether the output text names the edited files, for
example one line `edited src/main.rs (+3 -1)`, is undecided. The
recommendation is yes, in that short form, so the model and the reader agree
on what a call changed.

## Rationale

Watching the filesystem or diffing the working tree around a call cannot say
which call made a change, misses edits outside the repository, and blurs
concurrent calls. Notices from the write path are exact for everything that
runs in process and free of scanning. External processes are outside that path
by nature; their edits still appear in the working tree view, unattributed.

Only creations and modifications are reported because the conversation shows
what the model wrote, not the state of the directory. A move is a deletion the
view would not show and a creation it does show; reporting the destination as
`added` says what the reader needs.
