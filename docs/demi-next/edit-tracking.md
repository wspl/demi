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
they are logged to the runner/service diagnostic stream.

The shared recording implementation lives in `command-service/edits`: both the
runner and native command service use this file interface. The runner creates
the job's private `changes` directory next to its retained output, and provides
that directory and the installation's lock file as an `EditContext`. Its journal
and context have schemas in `command-protocol`; they are not another message
stream and never appear on command stdout or stderr.

Brush and `context::fs` enter this recorder for their write operations. Opened
regular files are registered with their identity so cloned descriptors, utility
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
retained output directory's lifetime ([Pipes and output](runner.md#pipes-and-output)).

Uncooperative external writers do not take this lock. Their concurrent writes
cannot be isolated by these hooks; they can affect a before/after capture.
Tracking guarantees coordination among the participating runner and native
command writes, not filesystem isolation from other applications.

## The report

`job_exit` carries `files` in first-change order and `filesTruncated`:

| Field | Meaning |
| --- | --- |
| `path` | Absolute, as the target names it. The browser displays it relative to the workspace root when under it, absolute otherwise, with `/` separators. |
| `kind` | `added` or `modified`. |
| `added`, `removed` | Total line additions and removals across this job's retained edit segments. A segment without contents contributes zero. |
| `edits` | Ordered segments for this file. Each has `original` and `modified` snapshot paths on the target. `original` is absent when that segment created the file. Both are absent when contents were not retained. |

The common file fields are the working tree's
([Runner](runner.md#working-tree)) without `deleted`, `renamed`, and `from`.
The edit sequence belongs only to the call's history.

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
S3 in deployment. Locally it uses the backend data directory.
`DEMI_CHANGE_STORE_CONFIG` selects a JSON file with `bucket`, `region`, optional
HTTPS `endpoint`, and optional `forcePathStyle` for S3; credentials use the AWS
SDK provider chain. Backend shutdown releases the storage client. Its objects
are bound to the conversation, not addressed by content:

```text
changes/<conversationId>/<commandId>/<n>/<edit>.original
changes/<conversationId>/<commandId>/<n>/<edit>.modified
```

`n` is the file's index and `edit` is its segment index. The objects belong to the
conversation's owner and live as long as the conversation; an archived
conversation keeps them. They are written before the block that lists them is
checkpointed, under the blob rule that a committed block never points at
unpublished bytes.

The transcript block's `ShellToolView.files` carries the list only:
`path`, `kind`, `added`, `removed`, and `edits`. Each edit has `kept`, true
when both sides are in the store. An added segment stores an empty original
side. A Fork copies the retained objects for its blocks into its own namespace
before checkpointing; a missing source object remains unavailable.

`GET /api/conversations/:id/commands/:commandId/changes/file?path=...&edit=0` returns
`{ original, modified }` for one segment from the store, `original` empty when
that segment created the file; 404 `not_found` when the command has no such
file/segment or its contents were not kept. No host is involved.

## Delivery to the conversation

The shell block shows the entries as file pills under the call, from the block's
view ([Tool rendering](../tool-rendering-spec.md)). Picking a pill opens the
work panel's change view in Conversation mode on that call, with the picked
file selected. Selecting the fixed Change section returns to Uncommitted,
including when Change is already selected. Its header counts always describe
the uncommitted working tree, not the retained edit. This mode receives only that file's metadata, command ID and
edit segments, not the call's file list. Its two sides, fetched through the
route above, show in the same diff editor the Uncommitted mode uses. If the file
has several edit segments, a shared control selects one in order, initially the
first. The selection identifies the command, file and segment, so opening another call for the same path replaces its diff.
An edit that is not `kept` has no diff; the interface silently omits the diff
without a message or explanation. Its file pill remains under the call.
Conversation mode has no file sidebar, list source or refresh operation. Picking
another pill replaces the selected edit; Back and Forward revisit selections.
Only Uncommitted mode lists files and offers a changed-file tree.

## Responsibilities

| Where | Responsibility |
| --- | --- |
| `vendor/brush-core`, `vendor/uucore`, `vendor/sed` | Route writable opens, file writes and temporary-file publication through the execution owner's hooks. |
| `packages/command-protocol`, `crates/command-service` | Define the recording context/journal and implement the shared bounded recorder with OS locking. |
| `crates/demi-commands` | Record create, edit, patch publication and rollback using the invocation's recorder. |
| `crates/runner` | Create job recording contexts, associate descriptors with paths, forward redirected external output, finalize reports and share line counting with the working tree. |
| `packages/runner-protocol`, `packages/shell`, `packages/host-remote` | Carry the report through command completion. |
| `packages/backend` | Publish snapshots before tool completion, retain conversation history, authorize reads and serve individual edit segments. |
| `packages/agent` | Carry the small file/segment list in the shell tool view, exclusively for the user. |
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
whole and a file is up to 8 MiB: they are read only when a file is opened, so
they live in a store that is fetched by entry. They are bound to the
conversation rather than content-addressed because they are the conversation's
history and go with it, and because two sides of one edit are only meaningful
together.

Only creations and modifications are reported because the conversation shows
what the model wrote, not the state of the directory.
