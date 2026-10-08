# Edit tracking

A shell job reports the files it created or modified, so the conversation can
show what one tool call edited: the list under the call, and each file's diff
on request. The runner learns this from the commands themselves as they run,
not from the filesystem afterwards: every in-process file write passes through
a layer the runner owns, and that layer takes notes. This is why the shell runs
in process and why the utilities are forked to open files through one context.
Native Demi file commands also participate in recording, as described below.

Edit tracking has one consumer: the conversation's view of what the agent
changed, the file pills under a shell call and the line under a request's
reply, and the work panel's change view in Conversation mode that they open.
Nothing else reads it. The model
never receives it; the working tree view ([Runner](runner.md#working-tree))
answers a different question, "what is uncommitted?", for the whole directory.
The two share the file entry shape and the diff view and nothing else.

The list travels with the transcript block. The file contents behind the diff
do not: they are blobs the list names ([Edit copies](#edit-copies)), and the
web app fetches them when a file is opened.

## Scope

| Recorded | Not recorded |
| --- | --- |
| A file a brush redirection opens for writing (`>`, `>>`, `<>`, `exec 3>f`). | Writes by external programs that do not participate in recording: git, python, node, user-installed tools. |
| A file an embedded utility opens for writing, writes whole, or renames over (`sed -i`, `tee`, `sort -o`, `uniq` with an output file). | Copies and hard links (`cp`), deletions, renames as moves, directories, permissions, ownership, times. |
| Files created or modified by `demi file edit` and `demi file patch`, including when one call changes several files. | Edits prepared by a native command but never written, or successfully rolled back. |
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
copies from the target, every copy of the command in one request and one pipe
([Host operations](runner.md#host-operations)), a copy two edits share once,
and stores each as a blob in the conversation owner's namespace, then hands
the tool its status with the list. The read is part of
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

For example, the user asks "fix the sign-in page". The agent edits
`login.ts` and `form.css`, starts a build and yields until it ends; woken,
it edits `login.ts` again and replies. Under the reply a line reads
*2 Files Changed +10 −0*. It opens the Change
view on that request: a sidebar lists `login.ts` and `form.css`, and
`login.ts` shows its change from before the first edit to after the second.

### A request

A request is what one message of the user asked for: it starts at a `user`
block, the message as sent or edited, and holds every block of the same
agent up to its next `user` block. What wakes the agent without the user
asking starts no request and belongs to the request it continues:

| Block that continues the work | Starts a request |
| --- | --- |
| `user`: a message sent, edited or regenerated | Yes |
| `steer`: the user adds to the running turn | No |
| `wakeup`: a yield's wakeup | No |
| `agent_message`: a receipt or a message from another agent, which can start a continuation | No |
| `resume`: Resume, or a turn continued after compaction or a model switch | No |

So a request's turns may be several, and its changes are those of all of
them. A message sent while the agent works waits in the queue and starts
its own request when it runs. The request is derived from the transcript
when shown, never stored.

### Each agent its own

A request belongs to one agent, and so do its changes: the changes a
subagent makes show in the subagent's own transcript, under its own
requests, which start at the `user` blocks of its spawn and its resumes,
and never under the parent's. The parent's transcript shows what the
parent's own commands changed.

### What the conversation shows

- **The pills.** Under a shell call, its files, from the block's view
  ([Rendering boundary](../agent/runtime.md#rendering-boundary)).
- **The request's line.** At the end of a request's reply, above its Copy
  and Fork, once one of its calls has changed a file, a button that names
  how many files and the lines added and removed across them:
  *2 Files Changed +10 −0*, as GitHub and Cursor sum a change. It grows as
  later calls of the request end. It counts files the request's calls
  changed and that still exist when each call ended. The lines are each
  file's All Changes, counted as the Change view counts them, not the sum of
  each call's counts, which counts a line twice when two calls change it:
  when the line comes into view, the page reads each file's first original
  and last result, which never change, so once a page. Until they are
  counted, or when a file's ends were not kept, the line shows the files
  alone.
- **The Change view.** The line and a pill open the `edit` intent, which the
  work panel's Change view opens in Conversation mode on the request
  ([Intents](../architecture/plugin-pages.md#intents)); with the `changes`
  plugin off, neither is a control. The sidebar lists the request's files in
  the order they were first changed. A file shows **All Changes**, from its
  contents before the request's first edit to after its last, and a control
  steps through each edit of the request in order, `Edit 2 of 3`, each named
  by its call's title. All Changes is offered when the first edit's original
  and the last edit's result were both kept; otherwise the file opens at its
  first edit with contents. The line opens the first file's All Changes; a
  pill opens its file at that call's first edit. A request that an edit or
  a regenerate removed says *These changes are no longer in the
  conversation.* Picking another file or edit
  replaces the selection; Back and Forward revisit selections. A file whose
  every edit is without contents is listed without a diff. Under the list,
  one line says that files other programs wrote are not here, with a link to
  Uncommitted mode, which shows everything uncommitted.
- **What it cannot show.** Edit tracking records only the writes that pass
  through it ([Scope](#scope)): a file `npm install` or `git checkout` wrote
  is not listed. When something outside the request changed a file between
  two of its edits, All Changes includes that change, as it spans from the
  first edit's original to the last edit's result; the edits one by one show
  only the request's own. An edit or a regenerate removes the replaced
  calls from the transcript, and their changes leave the list with them,
  though their files stay as they were written.

The Change tab keeps its mode and selection when the user leaves it and
comes back, as any tab keeps what it shows. Its header counts always
describe the uncommitted working tree, not the request. The two sides of an
edit come from the blob route ([Edit copies](#edit-copies)); All Changes
takes its original from the first edit and its result from the last. Only
Uncommitted mode lists the working tree's files and has a refresh.

## Responsibilities

| Where | Responsibility |
| --- | --- |
| `vendor/brush/brush-core`, `vendor/uutils-coreutils/uucore`, `vendor/uutils-sed/sed` | Route writable opens, file writes and temporary-file publication through the execution owner's hooks. |
| `crates/command-protocol` | Define the recording context, the journal and the test of whether bytes are text. |
| `crates/command-sdk` | Implement the shared bounded recorder with OS locking. |
| `crates/command-package-file` | Record create, edit, patch publication and rollback using the invocation's recorder. |
| `crates/runner-jobs`, `crates/runner-shell` | Create job recording contexts, associate descriptors with paths, forward redirected external output, finalize reports and share line counting with the working tree. |
| `crates/runner-protocol`, `crates/host-interface`, `crates/backend-remote-host` | Carry the report through command completion. |
| `crates/backend-user-shard` | Store the copies as blobs before tool completion; the blob route serves them. |
| `crates/shared-types`, `crates/agent-tools` | Define the shell tool view with its small file and segment list, and carry it in the transcript, exclusively for the user. |
| `packages/web-ui` | Deriving a request's changes from the transcript, the request's line, and shared file selection, edit selection and diff behavior. |
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
