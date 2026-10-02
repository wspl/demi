# Edit tracking

A shell job reports the files it created or modified, so the conversation can
show what one tool call edited: the list under the call, and each file's diff
on request. The runner learns this from the commands themselves as they run,
not from the filesystem afterwards: shell redirections pass through the
interpreter's open handler into a layer the runner owns, and that layer takes notes. This is why the shell runs in
process: it gives the runner a writable-open hook in every nested shell scope,
so redirections can use the shared recorder.
Native Demi file commands also participate in recording, as described below.

Edit tracking has one consumer: the file pills under a shell call in the
conversation, and the work panel's change view in Conversation mode that a
pill opens to show what that call changed. Nothing else reads it. The model
never receives it; the working tree view ([Runner](runner.md#working-tree))
answers a different question, "what is uncommitted?", for the whole directory.
The two share the file entry shape and the diff view and nothing else.

The list travels with the transcript block. The file contents behind the diff
do not: they are blobs the list names ([Edit copies](#edit-copies)), and the
web app fetches them when a file is opened.

## Scope

| Recorded | Not recorded |
| --- | --- |
| Files opened for writing by redirections (`>`, `>>`, `<>`, `exec 3>f`) through the fork's open handler in every in-process scope: top level, subshells, functions, background and nested background tasks, command substitutions and (on Unix) process substitutions. | Files external programs open themselves, including git, python, node, user-installed tools and system utilities such as `sed -i`, `tee`, `sort -o` and `uniq` with an output file; and what an external program writes through a read-write descriptor it inherits (`3<>f`), which stays the native file so the program can read and seek it, while the shell's own writes through it are recorded. |
| Writes from external stdout and stderr redirected by the shell, forwarded through the recorder. | Copies and hard links (`cp`), deletions, renames as moves, directories, permissions, ownership, times, reads, and the empty file `mktemp` creates. |
| Files created or modified by `demi file create`, `demi file edit`, and `demi file patch`, including when one patch edits several files. | Edits prepared by a native command but never written, or successfully rolled back. |

A file is `added` when it did not exist before this job first changed it,
`modified` otherwise. A path absent when the job finishes is omitted, whether
it existed before the job or was created during it. An operation with identical
captured before and after bytes is omitted. A successfully rolled-back native
patch is also omitted.

| Limit | Value | Beyond it |
| --- | --- | --- |
| File size, before or after | 8 MiB | The file remains in the list, with no diff or line counts for that edit. Binary and non-UTF-8 content are treated the same. |
| Snapshot bytes written per job | 64 MiB | Further edits have no contents. This includes replacement snapshots when successive writes are combined. |
| Recorded paths per job | 500 | Later paths go unrecorded; `filesTruncated` is true. |
| Edit segments per job | 1,000 | Further edits retain the file entry without contents; `filesTruncated` is true. |

## Recording actual writes

For example, A changes the first line, B changes the second, and A changes the
first again. A must show its two edits without attributing B's second-line edit
to A:

```text
             x=0, y=0
A writes     x=1, y=0     A edit 1: x=0 -> x=1
B writes     x=1, y=1
A writes     x=2, y=1     A edit 2: x=1 -> x=2
```

The recorder takes the before and after contents around each actual filesystem
mutation: a write-capable open, a write to an opened file, a whole-file write,
or publishing a temporary file over a destination. It saves the after contents
before releasing that operation, not by rereading the workspace at job exit.
A failed operation is also examined, since it may have changed some bytes.

All participating writes in one runner installation take the same short-lived
OS file lock. Under that lock, the recorder reads the before contents, performs
the mutation, reads the after contents, and publishes its snapshots and journal.
The lock is held for a single file operation, never while a command waits for
input, another command, or a background task. Pipes, devices and directories do
not enter the recorder. Closing the lock file releases it on normal return,
unwinding or process death. Recording failures never change the command result;
they go to the [Host log](runner.md#host-log): the runner writes its own as log
events, and a native service writes its own to its standard error, which the
runner drains into the log.

The shared recorder belongs to the command-sdk library, which the runner
and the native command services both link, so both record through the same
file interface. The runner creates the job's private `changes` directory in
the job's directory, next to its kept output, and provides that directory and
the installation's lock file as an `EditContext`. The journal and the context are types of the command
wire; they are not another message stream and never appear on command stdout or
stderr.

The fork's open handler supplies recorded files for shell redirections, and
native Demi file commands enter the same recorder for their write operations.
Opened regular files are registered with their identity so cloned descriptors, builtin
stdout and shell descriptor duplication retain their association with the path.
Writes to an old descriptor after its path was replaced do not edit the file now
at that path and are not attributed to it. Shell redirects of external stdout
and stderr use runner-owned pipes that forward into the registered file through
the same recorder. The child command completes only after these forwarded streams have
drained; a forwarding error fails that command. Other filesystem writes performed
inside external programs remain outside tracking, including writes through their
other inherited file descriptors.

A temporary-file replacement records the destination before any backup rename
or Windows replacement removal, and the final destination afterwards. Temporary
names and backup moves are not edits. Native `file.create`, `file.edit`, and
`file.patch` use this same recording operation for publishing their temporary
files and restoring earlier contents after a failed patch. A patch that fully
rolls back leaves no edit; failed rollback preserves the changes still present.
The runner supplies `EditContext` from the authenticated live execution context,
not from command arguments or an external forwarding client's metadata.

Successive writes to one file are combined only when the previous saved after
contents equal the next before contents. The first before copy and latest after
copy then describe those writes without including anyone else's changes. If
those versions differ, a new segment is retained. This also combines buffer
writes and a truncating open followed by writes. When a combined segment returns
to its original bytes, it is removed. A segment without contents cannot establish
continuity: that file subsequently retains only metadata, rather than a stale or
misleading diff. Line counts sum the retained segments, using the working-tree
diff algorithm.

At job completion, after all job-owned work has stopped, the runner reads the
journal, omits absent paths and empty edits, and sends the report. It never uses
the current file bytes as an edit's after contents. Journal and copies share the
job directory's lifetime: they go once the backend has read the job's end
([Pipes and output](runner.md#pipes-and-output)).

Uncooperative external writers do not take this lock. Their concurrent writes
cannot be isolated by these hooks; they can affect a before/after capture.
Tracking guarantees coordination among the participating runner and native
command writes, not filesystem isolation from other applications.

## The report

`job_exit` carries `files` in first-change order and `filesTruncated`:

| Field | Meaning |
| --- | --- |
| `path` | Absolute, as the target names it. The web app displays it relative to the workspace root when under it, absolute otherwise, with `/` separators. |
| `kind` | `added` or `modified`. |
| `added`, `removed` | Total line additions and removals across this job's retained edit segments. A segment without contents contributes zero. |
| `edits` | Ordered segments for this file. Each has `original` and `modified` snapshot paths on the target. `original` is absent when that segment created the file. Both are absent when contents were not retained. |

The common file fields are the working tree's
([Runner](runner.md#working-tree)) without `deleted`, `renamed`, and `from`.
The edit sequence belongs only to the call's history.

## Edit copies

When a command's exit reaches the backend, the backend reads each entry's
copies from the target and stores each as a blob in the conversation owner's
namespace, then hands the tool its status with the list. The read is part of
completing the command inside the tool call's host access, the same way the
command's kept output is read back; it is not a separate operation on the Host
from outside the agent ([Host operations](sessions-and-targets.md#host-operations)).
A copy that cannot be read or stored leaves its entry in the list without
copies; the list is never lost over the copies.

The shell call's view in the transcript block carries the list, in its `files`
field ([Transcript](../agent/runtime.md#transcript)): `path`, `kind`, `added`,
`removed`, and `edits`. Each edit has `copies`, the blobs of the file's two
sides, `original` before the edit and `modified` after it, or none when they
were not stored. An added segment's original is the empty blob. For example, a
command that edits `main.rs` in two segments stores three blobs: the file
before the first segment, between the two, and after the second, since the
first segment's modified side is the second's original.

A block references its copies as it references its media, so the blob rules
of [Storage](../backend/storage.md#attachment-and-transcript-media) hold for
them: a copy is stored before the block that lists it is checkpointed, stays
while a block references it, which is as long as the conversation, archived
or not, and is collected once nothing does. A Fork copies no bytes: its blocks
reference the same blobs of the same namespace.

The web app reads one segment's two sides from the blob route
([Uploads and media](../product/web-api.md#uploads-and-media)) by the hashes in
the view, without involving the Host; an added segment's original is empty.

## Delivery to the conversation

The shell block shows the entries as file pills under the call, from the block's
view ([Rendering boundary](../agent/runtime.md#rendering-boundary)). Picking a
pill opens the `edit` intent, which the work panel's Change view opens in
Conversation mode on that call, with the picked file selected
([Intents](../architecture/plugin-pages.md#intents)); with the `changes`
plugin off, the pills are not controls. Picking the Change tab in the strip
returns to Uncommitted, including when it is already selected. Its header counts always
describe the uncommitted working tree, not the retained edit. This mode receives only that file's metadata, command ID and
edit segments, not the call's file list. Its two sides, fetched from the blob
route, show in the same diff editor the Uncommitted mode uses. If the file
has several edit segments, a shared control selects one in order, initially the
first. The selection identifies the command, file and segment, so opening another call for the same path replaces its diff.
An edit without copies has no diff; the interface silently omits the diff
without a message or explanation. Its file pill remains under the call.
Conversation mode has no file sidebar, list source or refresh operation. Picking
another pill replaces the selected edit; Back and Forward revisit selections.
Only Uncommitted mode lists files and offers a changed-file tree.

## Responsibilities

| Where | Responsibility |
| --- | --- |
| `third_party/mvdan-sh` | Route writable redirections in every in-process scope through the job's open handler, preserving it in nested interpreters and executable fallback. |
| `crates/command-protocol` | Define the recording context, the journal and the test of whether bytes are text. |
| `crates/command-sdk` | Implement the shared bounded recorder with OS locking. |
| `crates/command-package-file` | Record create, edit, patch publication and rollback using the invocation's recorder. |
| `crates/runner-jobs`, `crates/runner-shell` | Create job recording contexts, associate descriptors with paths, forward redirected external output, finalize reports and share line counting with the working tree. |
| `crates/runner-protocol`, `crates/host-interface`, `crates/backend-remote-host` | Carry the report through command completion. |
| `crates/backend-user-shard` | Store the copies as blobs before tool completion; the blob route serves them. |
| `crates/shared-types`, `crates/agent-tools` | Define the shell tool view with its small file and segment list, and carry it in the transcript, exclusively for the user. |
| `packages/web-ui` | Shared file selection, segment selection and diff behavior. |
| `packages/web`, `packages/web-gallery` | Product data adapters and matching specimens. |

## Rationale

Watching the filesystem or diffing the working tree around a call cannot say
which call made a change, misses edits outside the repository, and blurs
concurrent calls. Notices from the write path identify which job attempts a
write without scanning the directory. Native Demi commands participate explicitly
through the invocation recording context. Other external programs remain outside that
path; their edits still appear in the working tree view, unattributed.

The list is in the block because it is small and is read every time the
conversation is shown. The contents are not, because a transcript is loaded
whole and a file is up to 8 MiB: they are read only when a file is opened.
They are blobs, addressed by their content like every other byte a
conversation keeps. The block's references still bind them to the
conversation's history and pair an edit's two sides; the collector removes
them with the last reference, as it removes media; and a version of a file is
stored once, however many segments and Forks name it.

Only creations and modifications are reported because the conversation shows
what the model wrote, not the state of the directory.

Binary contents are never retained, whatever their size: copies of images and
media would fill the blob store for little use, since the change view's
history is about text. The work panel shows a binary file only as it exists
on the Host ([File previews](../product/file-previews.md)).

## Open items

- **Pending:** Restore edit tracking for files that system utilities such as
  `sed -i`, `tee` and `sort -o` open and write themselves. These writes are
  outside the recorder in this migration. A shell redirection such as
  `sort input > output` is still recorded; `sort -o output input` is not.
