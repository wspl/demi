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
| A file an embedded utility opens for writing, writes whole, or renames over (`sed -i`, `tee`, `sort -o`, `uniq` with an output file, `mv` onto an existing file). | Copies and hard links (`cp`), deletions and a rename of a file the job did not change (a move), as edits: the report lists them apart ([The report](#the-report)); directories, permissions, ownership, times. |
| The edits the job recorded under a file's old name, carried to its new name when an embedded `mv` renames it. | |
| Files created or modified by `demi file edit` and `demi file patch`, including when one call changes several files. | Edits prepared by a native command but never written, or successfully rolled back. |
| Every in-process part of the job: subshells, functions, background tasks, process substitutions. | Reads, and the empty file `mktemp` creates. |

### Within the working directory

What the conversation shows as changed is what the agent changed in its
working directory, the directory its commands start in: the root's is the
conversation's, a subagent's its own
([Working directory](../agent/subagents.md#working-directory)). For example,
the agent works in `~/shop`, writes `/tmp/fix.py`, runs it, edits
`~/shop/src/cart.ts` and `~/.zshrc`: the pills and the Change view show
`src/cart.ts` alone. Commits mode reads the same directory
([Working tree](runner.md#working-tree)), so both modes show the same part
of the Host.

- **A file outside the working directory** the agent's call started in is
  not shown, wherever it is: a scratch file, the conversation's scratch
  directory, a file in `~` or `/etc`. The runner records every edit as
  before, so a later `mv /tmp/page.ts.new src/page.ts` still carries them to
  `src/page.ts`; the backend, which computes every list
  ([What the conversation shows](#what-the-conversation-shows)), leaves out
  a path outside the directory, as the request lists it after its renames.
  A path is compared by its parts after `.` and `..` are resolved, in the
  Host's own spelling, case and `\` included on Windows, so
  `~/shop/../.zshrc` is outside `~/shop` and `C:\Users\u\shop\src` is
  inside `C:/Users/u/shop`; a symbolic link is not followed. A call's files
  are judged against the directory it started in, once, when it is stored,
  and a later call of the request never judges them again, so a `demi agent
  cwd` between two calls changes only what follows.
- **A file the request created and removed** is not in the request's list
  in the Change view, which shows what the request left behind. A call's
  pills show what that call did and never change after it: the call that
  wrote the file shows it, and the call that removed it shows it struck
  through. A run's pills compare the run's start with its end, so a file the
  run both created and removed is at neither end and shows in none of them;
  one created in a run and removed in a later run still shows in the first
  run's pills. For example, a request's first call creates `notes.md` and its
  fourth removes it: opened, the first call's pills show `notes.md` and the
  fourth's show it struck through; folded, a run holding both shows neither,
  and the Change view does not list it. A file that was there before the
  request and that it removed shows struck through everywhere.
- A file removed by a program the runner does not record, such as `python`
  deleting it, cannot be told from one that stays, and stays shown.
- Nothing else is hidden: a file the project's git ignores, such as `.env`
  or a log, shows when the agent wrote it, since git's ignore list says what
  is not committed, not what the user does not review.

A file is `added` when it did not exist before this job first changed it,
`modified` otherwise. A path absent when the job finishes is omitted, whether
it existed before the job or was created during it. An operation with identical
captured before and after bytes is omitted. A successfully rolled-back native
patch is also omitted.

For example, a job writes `/tmp/x` and runs `mv /tmp/x src/new.ts`:
`src/new.ts` is `added` with the contents written, and `/tmp/x`, gone, is
omitted. A job that runs `mv /tmp/x src/page.ts` over an existing
`src/page.ts` modifies it, from its old contents to the new. A job that only
runs `mv a.ts b.ts` records nothing, since it changed no file's contents. A
renamed folder carries the edits recorded under each file in it: an append
to `old/x.ts` followed by `mv old moved` shows as `moved/x.ts`.

| Limit | Value | Beyond it |
| --- | --- | --- |
| File size, before or after | 8 MiB | The file remains in the list, with no diff or line counts for that edit. Binary content, by the one rule ([What `demi file view` shows](../agent/runtime.md#what-demi-file-view-shows)), is treated the same; other bytes are text. |
| Snapshot bytes written per job | 64 MiB | Further edits have no contents. This includes replacement snapshots when successive writes are combined. |
| Recorded paths per job | 500 | Later paths go unrecorded; `filesTruncated` is true. |
| Renames and removals per job | 500 | Later ones go unlisted; `filesTruncated` is true. |
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
names and backup moves are not edits. Native `file.edit` and `file.patch` use this
same recording operation for publishing their temporary files and restoring
earlier contents after a failed write. A patch that fully
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

`job_exit` carries `files` in first-change order, `pathChanges` and
`filesTruncated`. `pathChanges` lists the renames and removals the job's
embedded utilities made, in the order they happened: `renamed` with `from`
and `to`, whether or not the job changed the file, and `removed` with
`path`, once per argument of `rm`, so `rm -r scratch/` is one entry. A
removal also clears the job's own edits at or under its path, so a file made
there again starts anew. A path the job itself created and then removed is
not listed, since nothing of it was there before or is there after, as
`printf x > scratch.txt; rm scratch.txt` lists nothing. Each entry of `files`:

| Field | Meaning |
| --- | --- |
| `path` | Absolute, as the target names it. The web app displays it relative to the workspace root when under it, absolute otherwise, with `/` separators. |
| `kind` | `added` or `modified`. |
| `added`, `removed` | Total line additions and removals across this job's retained edit segments. A segment without contents contributes zero. |
| `edits` | Ordered segments for this file. Each has `original` and `modified` snapshot paths on the target. `original` is absent when that segment created the file. Both are absent when contents were not retained. |

The call's view keeps these files and adds, from `pathChanges`, each file the
call removed as `deleted` and each it renamed as `renamed` with the old path
in `from`, the kinds the working tree uses ([Runner](runner.md#working-tree)),
so a call's pills show what it removed and moved. The edit sequence belongs
only to the call's history.

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
`login.ts` and `form.css`, starts a build and ends its turn; woken by the
build's end,
it edits `login.ts` again and replies. The pills under its work name
`login.ts` and `form.css`; a click on `login.ts` opens the Change view on
that request: a sidebar lists both files, and `login.ts` shows its change
from before the first edit to after the second.

### A request

A request is what one message of the user asked for: it starts at a `user`
block, the message as sent or edited, and holds every block of the same
agent up to its next `user` block. What wakes the agent without the user
asking starts no request and belongs to the request it continues:

| Block that continues the work | Starts a request |
| --- | --- |
| `user`: a message sent, edited or regenerated | Yes |
| `steer`: the user adds to the running turn | No |
| `wakeup`: command reports | No |
| `agent_message`: a receipt, a message from another agent, or the user's permission decision or move notice, which can start a continuation | No |
| `resume`: Resume, or a turn continued after compaction or a model switch | No |

So a request's turns may be several, and its changes are those of all of
them. The request's files are listed as they stand after its latest call: a
file that a later call renamed with an embedded `mv` is listed under its new
name, its earlier edits with it; a file that was there before the request
and that a later call removed is listed `deleted`, struck through; and a
file the request created and a later call removed is not listed. For example, one call
writes `/tmp/page.ts.new` and the next runs `mv /tmp/page.ts.new
src/page.ts`: the request lists `src/page.ts`, modified, and never
`page.ts.new`. A call's report names, beside its edits, the renames and
removals its embedded utilities made ([The report](#the-report)), and the
request applies each call's before adding the call's own files:

- A file a rename brought to a name the request had not listed is `renamed`,
  with its old path, when it was there before the request, and `added` when
  the request created it, unless the renaming call lists that name as
  `modified`, as when it replaced an existing file. A rename onto a listed file merges into it,
  which keeps its kind and the earlier of the two places in the list.
- An edit a later call's rename carried shows as it was made, its two sides
  those of the file it then was, and the view's heading names both paths,
  `/tmp/page.ts.new → src/page.ts`.
- All Changes of a file that was there before the request starts from the
  original of its first edit that did not create its file, so a replaced
  page shows the old page against the new; of a file the request created,
  from its first edit.
- A call's own pills still show what that call did. A pill opens its file
  under its current name, and a pill for a file a later call of the request
  removed is no control.

A message sent while the agent works waits in the queue and starts
its own request when it runs. The backend derives the request's list by
these rules as it stores each call, into the call's `requestFiles`
([What the conversation shows](#what-the-conversation-shows)); the page
derives nothing.

### Each agent its own

A request belongs to one agent, and so do its changes: the changes a
subagent makes show in the subagent's own transcript, under its own
requests, which start at the `user` blocks of its spawn and its resumes,
and never under the parent's. The parent's transcript shows what the
parent's own commands changed.

### What the conversation shows

- **What a page receives, it shows; it counts nothing.** Every list of
  changed files and every line count the conversation shows is computed
  once by the backend, when it stores a call's block, and stored in the
  call's view ([Rendering boundary](../agent/runtime.md#rendering-boundary)):
  the backend holds the edit copies it just stored and the whole transcript,
  so it reads no blob twice and needs no page of the transcript the page has
  not read. A list once stored never changes: where a run or a request
  starts is decided by the blocks before a call, and later blocks only end
  it, while an edit or a regenerate removes the later blocks whole. Calls of
  one step may return in any order: a call's lists are final once every call
  before it has returned, and change at most once before then. Lines are
  counted by one algorithm, gix's blob diff (`imara-diff`), for the runner's
  report and the backend's lists alike.
- **The pills.** Under a shell call, its files, from the block's view: what
  the call wrote, removed (struck through) and renamed (under its new name,
  the old one in its tooltip). Under a folded work group
  ([Work groups](../agent/runtime.md#work-groups)), the run's files as the
  latest call of the run stored them in `runFiles`: each file once, in the
  order first changed, its kind, and the lines added and removed between
  its contents before the run and after that call, so a line two calls
  changed counts once; a file whose two ends were not kept is named alone,
  and a file the run created and removed is at neither end and not listed.
  The pills show with their counts the moment the block does. A pill opens
  its file at the first of the run's calls that changed it. The pills take
  at most two lines; the files that do not fit fold into a last pill, *+12
  Files*, which opens the Change view on the call or run at the first file
it holds, as a pill does,
  where every file is listed, so a call that removed sixty screenshots
  reads as a few pills and a count.
- **The Change view.** A pill opens the `edit` intent, which the work panel's
  Change view opens in Conversation mode on the request
  ([Intents](../architecture/plugin-pages.md#intents)); with the `changes`
  plugin off, a pill is no control. The sidebar shows the request's files as
  the request's latest call stored them in `requestFiles`, as a tree of the
  working directory, the way VS Code's Source Control shows changes as a
  tree: folders that hold a changed file, folders first and names in order,
  a folder whose only content is one folder joined with it into one row
  (`src/web/app`), every folder open; each file with its kind (added, modified, deleted, or
  renamed with its old path), All Changes' line counts, and its edits, each
  naming its call and the two edit copies, so the page reads copies only to
  draw a diff, never to count. A file outside the agent's working directory, or one the
  request created and removed, is not listed
  ([Within the working directory](#within-the-working-directory)). A file shows **All Changes**, from its
  contents before the request's first edit to after its last, and a control
  steps through each edit of the request in order, `Edit 2 of 3`, each named
  by its call's title. All Changes is offered when the first edit's original
  and the last edit's result were both kept; otherwise the file opens at its
  first edit with contents. A pill opens its file at that call's first
  edit. A request that an edit or
  a regenerate removed says *These changes are no longer in the
  conversation.* Picking another file or edit
  replaces the selection; Back and Forward revisit selections. A file whose
  every edit is without contents is listed without a diff.
- **What it cannot show.** Edit tracking records only the writes that pass
  through it ([Scope](#scope)): a file `npm install` or `git checkout` wrote
  is not listed. When something outside the request changed a file between
  two of its edits, All Changes includes that change, as it spans from the
  first edit's original to the last edit's result; the edits one by one show
  only the request's own. An edit or a regenerate removes the replaced
  calls from the transcript, and their changes leave the list with them,
  though their files stay as they were written.

### Commits mode

The Change view's other mode, **Commits**, shows the repository the working
directory is in, as VS Code's Source Control and its commit list do
together. For example, Alex opens it on `~/shop`, checked out on
`feat/cart`:

```text
┌ Change ───────────────── [Commits | Conversation]  [⎇ feat/cart ▾] [⇤] [▤] ┐
│                                    │ src/                                  │
│   the selected file's diff         │   cart.ts                 +3 −1  M    │
│                                    │ docs/guide.md             +12    M    │
│                                    ├──────────────── ═ ────────────────────┤
│                                    │ ● Uncommitted Changes          3 files│
│                                    │   Fix the cart total     Alex · 2h    │
│                                    │   Add the cart page      Sam · 1d     │
└────────────────────────────────────┴───────────────────────────────────────┘
```

- **The sidebar** is two panes with a divider the user drags, its place kept
  as the sidebar's width is: above, the selected item's files, as the tree
  of [What the conversation shows](#what-the-conversation-shows); below, the
  **commit list**.
- **The commit list** shows one branch's history, newest first, following
  first parents, each commit its summary, author and age, *2h*, its full
  message, id and time in its tooltip; more load as the list scrolls to its
  end. While the branch shown is the one checked out, the list starts with
  **Uncommitted Changes** and its count of files, the working tree against
  HEAD as before; another branch's list starts with its newest commit. A
  repository without commits shows Uncommitted Changes alone; outside a
  repository the mode says *Not a Git repository*.
- **Selecting** an item shows its files above and opens the first; a commit
  shows its changes against its first parent, a root commit everything
  added. Back and Forward revisit selections, as for files.
- **The branch menu** in the toolbar names the branch the list shows, the
  checked-out one at first, *HEAD (3f2a9c1)* when HEAD is detached, and lists
  the local branches, most recently committed first, the checked-out one
  marked. Choosing one only changes what the list shows: nothing on the Host
  is checked out. Remote branches and the branches' graph are not shown.
- **It follows the repository.** A commit, a checkout or a new branch made
  anywhere, in the agent's shell or the user's terminal, changes the list
  and the menu at once, from the same watch that announces the working
  tree's changes ([Working tree](runner.md#working-tree)). When the branch
  shown is deleted, the list returns to the checked-out one.
- **A commit link**, `commit:3f2a9c1` in a message
  ([Links](../product/file-previews.md)), opens the Change view in Commits
  mode on that commit, in the repository of the agent that wrote it, in its
  working directory when it wrote it. The list shows the branch it is on:
  the one shown when that has it, else the checked-out one, else the most
  recently committed branch that has it. A commit on no local branch shows
  alone, its files above and the list replaced by *This commit is on no
  branch*; one the repository does not have says *No commit 3f2a9c1 in
  ~/shop*. The agent's guide says how to link a commit
  ([System prompt](../agent/system-prompt.md)).

The Change tab keeps its mode and selection when the user leaves it and
comes back, as any tab keeps what it shows. Its header counts always
describe the uncommitted working tree, not the request. The two sides of an
edit come from the blob route ([Edit copies](#edit-copies)); All Changes
takes its original from the first edit and its result from the last. Only
Commits mode lists the working tree's files and has a refresh.

## Responsibilities

| Where | Responsibility |
| --- | --- |
| `vendor/brush/brush-core`, `vendor/uutils-coreutils/uucore`, `vendor/uutils-sed/sed` | Route writable opens, file writes and temporary-file publication through the execution owner's hooks. |
| `crates/command-protocol` | Define the recording context, the journal, the test of whether bytes are text, and the line counts of a change (`imara-diff`), which the runner's report and the backend's lists share. |
| `crates/command-sdk` | Implement the shared bounded recorder with OS locking. |
| `crates/command-package-file` | Record create, edit, patch publication and rollback using the invocation's recorder. |
| `crates/runner-jobs`, `crates/runner-shell` | Create job recording contexts, associate descriptors with paths, forward redirected external output, finalize reports and share line counting with the working tree. |
| `crates/runner-protocol`, `crates/host-interface`, `crates/backend-remote-host` | Carry the report through command completion. |
| `crates/agent-store` | The run and request rules and the lists of changed files, `runFiles` and `requestFiles`, computed from the copies through a blob reader; the session's completion of a call and the block format 3 migration both call it. |
| `crates/agent-transcript`, `crates/agent-session` | Set each step block's `run` as it is appended; compute a shell call's lists before completing it. |
| `crates/backend-user-shard` | Store the copies as blobs before tool completion; the blob route serves them. Supply the blob reader the lists and the migration read through. |
| `crates/shared-types`, `crates/agent-tools` | Define the shell tool view with its small file and segment list, and carry it in the transcript, exclusively for the user. |
| `packages/web-ui` | The file pills and the Change view's request list as the stored lists give them, and shared file selection, edit selection and diff behavior; it derives no list and counts no line. |
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

Binary contents, by the one rule a job's stdout is judged by
([What `demi file view` shows](../agent/runtime.md#what-demi-file-view-shows)),
are never retained, whatever their size: copies of images and
media would fill the blob store for little use, since the change view's
history is about text. The work panel shows a binary file only as it exists
on the Host ([File previews](../product/file-previews.md)).
