---
status: proposed
---

# A device's upload is sent once, in turn, and resumes where it broke

## Problem

Alex's agent makes twenty variants of a sticker at once. It runs twenty
`demi image edit` commands on Alex's laptop, each with the same two reference
images, `style.png` (0.7 MB) and `mascot.png` (1.4 MB):

```sh
for n in $(seq 20); do
  demi image edit "$prompt" style.png mascot.png --model example-image-2 > v$n.png &
done; wait
```

The model is called by the backend, so each command first has its runner send
the two images to the backend. The laptop reaches the backend through a local
proxy app, with about 175 KB/s up. Twenty uploads start together, each of
both files, 42 MB in all:

- The proxy's tunnel stalls: an upload sends its first 64 KB and nothing more
  for 15 seconds, then its connection breaks. Nineteen commands of twenty
  fail with `demi image edit: pipe failed: error sending request for url
  (https://demi.example.com/api/pipes/…)`, and each leaves a 0-byte file.
- The runner's own connection gets nothing through for 60 seconds either, so
  the backend counts the laptop as gone. The laptop shows offline for two
  minutes, and the commands of every conversation on it wait.
- The message names no cause. The runner and the backend log the same line,
  so nobody can tell a reset connection from a refusal.

The same uploads one at a time all succeed, and a fast link takes the twenty
at once. A slow link is not what breaks them: the runner sends the same bytes
twenty times at once, and gives up at the first break.

## Decision

A device's runner sends each upload in turn, never the same content twice,
and resumes an upload where its connection broke. What fails says why.

### What the user and the agent see

- The twenty commands of the example send `style.png` and `mascot.png` once
  each. The other nineteen commands send nothing: they use the content the
  backend already holds or is receiving. The laptop stays online.
- A command whose upload waits in the queue takes longer and says nothing
  about it; its output and exit are as before.
- A file the user opens in the Files panel, or a diff in the Changes panel,
  goes ahead of uploads that commands wait for, so a panel never waits behind
  an agent's batch.
- A failure names its cause, with every error beneath it:

  ```text
  demi image edit: pipe failed: error sending request for url
  (https://demi.example.com/api/pipes/…): client error (SendRequest):
  http2 error: stream error received: unspecific protocol error detected
  ```

### Which data is an upload

An upload is a whole piece of data the backend asks a device for, which the
runner can send again from any offset:

| What the backend reads                         | Example                                                          |
| ---------------------------------------------- | ---------------------------------------------------------------- |
| A file (`fs_readFile`)                         | the Files panel, a download, `demi attachment upload`, a command's input image |
| Several files (`fs_readFiles`)                 | an edit's copies, a workflow's file                              |
| A file at a commit (`git_show`)                | the Changes panel                                                |
| A command's kept output (`job_read`)           | `demi shell output`                                              |
| The media a command returned (`job_media_read`) | the image `demi file view` shows the agent                      |

The rest is not queued:

- `fs_look`'s answers and `fs_search`'s results are small: a read by
  content looks with no bytes, and reads its file as above.
- A stream goes as it is made: a command's standard input and output,
  `demi host shell`'s pipes, a port forward, an exposed port, a plugin's
  stream. A queue would stall the program or the page that waits on it. The
  link shares itself among these as it does among any connections.

### Sent once

`demi attachment upload` already copies a file by its content
(`crates/backend-host-access/src/attachment_commands.rs`): the Host hashes the
file (`fs_hashFile`), and its bytes travel only when the user's blob store
(`crates/backend-blobs/src/blobs.rs`) lacks them. A command's input image
goes the same way, and both now also wait for a read of the same content
already under way:

1. A plugin asks for a file by its content (`HostRead::content`), as
   `demi image edit` and `demi video` ask for their images.
2. The Host hashes it, and the backend reads it into the user's blob store
   only when the store lacks it and no read of the same SHA-256 is under way
   for the user (`UserBlobs::fetch_once`). A read that comes meanwhile waits
   for that one, and takes the bytes from the store.
3. Bytes that are not the SHA-256's fail the read: the file changed since it
   was hashed.

A content written this way is a blob nothing names, so the store's day of
grace (`GRACE_MS`) keeps it for the next read, after which the collection
removes it as it removes any unnamed blob. Docker's transfer manager joins a
transfer already running for the same key, and Git LFS's batch request asks
only for the objects the server lacks; this does both.

The Files and Changes panels keep their own check, the file's version
(`FsReadFile::version`), and do not go by content.

### In turn

- `PipeClient` (`crates/runner-process/src/pipes/uploads.rs`) owns one queue
  for the runner. It sends at most five uploads at once:

  ```rust
  /// The most uploads a runner sends at once, Docker's default for all of a
  /// daemon's pushes: more at once on a slow link through a proxy stalls
  /// them all, and the runner's own connection with them.
  const UPLOADS_AT_ONCE: usize = 5;
  ```

- The pipe says whether a page waits to show what goes into it, which the
  backend sets by who asked:

  ```rust
  pub struct PipeRef {
      pub id: String,
      pub url: String,
      /// Whether a page waits to show what the runner uploads into the
      /// pipe: the Files and Changes panels, a download.
      pub shown: bool,
  }
  ```

  A shown upload goes before every other; within each they go first come,
  first sent.
- A queued upload's pipe does not run out its arrival window
  (`crates/backend-remote-host/src/pipes.rs`, `ARRIVAL`): the runner reports
  `pipe_queued { pipe }`, and the pipe waits for its source until the source
  arrives or the runner's link ends.

### Resumes

The backend takes an upload in chunks, and a broken chunk is sent again from
the last byte the backend took, as the tus protocol's `Upload-Offset` does:

- The runner sends an upload as `PUT /api/pipes/{id}` requests of at most
  8 MiB each, the chunk Google Cloud Storage's and the AWS CLI's uploads use.
  Each names its offset in `Upload-Offset`, and the last says
  `Upload-Complete: ?1`, as the IETF draft of resumable uploads names them.
- The backend answers each chunk with the offset it has taken, and
  `HEAD /api/pipes/{id}` answers the same at any time.
- A chunk whose connection breaks is sent again on a new connection from the
  offset `HEAD` answers, for up to a minute without a byte taken
  (`RECONNECT_FOR`), as a download's connections are tried again
  (`crates/shared-artifacts/src/download.rs`).
- A connection that stalls without closing breaks as well: the runner's
  HTTP/2 connections ping the backend every 15 seconds and end when an
  answer takes over 20, as gRPC's keepalive does. Otherwise a stalled
  request would hold its turn, and every upload behind it, for good.
- The source of an upload reads from any offset: the runner keeps a file
  open and reads each range by position, so a read that still runs for a
  broken chunk never moves the next one's; a commit's file is bytes it
  holds. A file that got shorter fails the upload at once.

The edge buffers a request's body before the backend sees it, so a single
request that breaks at 90% gives the backend nothing; chunks keep what was
taken.

### What a failure says, and the logs

- An upload's error is its cause with every error beneath it, joined by `: `,
  in the command's message, in `pipe_done`, and in the backend's
  `pipe.failed`.
- The runner logs `upload.queued`, `upload.started`, `upload.resumed`,
  `upload.ended` and `upload.failed`, with the pipe, its priority, its bytes,
  the time it waited and the time it took. The backend logs `upload.skipped`
  for a content it already held or had coming.

### Releases

A backend serves only runners of its own release
(`crates/backend-http/src/runners.rs`), so no runner meets a backend that
takes no chunks, and no backend a runner that sends no hashes.

## Spike

- **A slow link alone breaks nothing.** From a desktop, 32 uploads at once,
  each held to 3 KB/s, went through the edge to the backend for six minutes;
  all 32 were answered.
- **The edge buffers the body.** On the laptop's 175 KB/s link, the backend
  took the uploads that succeeded, 0.3 MB to 2.3 MB each, in 135 ms at the
  median and 937 ms at most: their bytes had reached the edge before the
  backend saw the request. A resumed upload therefore has to be sent in
  chunks.
- **The tunnel stalls at once, not slowly.** Through the laptop's proxy,
  `curl` sending 32 uploads at once, over one HTTP/2 connection or over 32
  HTTP/1.1 ones, got 6 to 8 through; the rest broke. With six commands at
  once, the agent's later batches went through.

## Prior art

- Docker's transfer manager (`daemon/internal/distribution/xfer/transfer.go`
  in [moby/moby](https://github.com/moby/moby)) sends at most
  `max-concurrent-uploads` layers at once, 5 by default across all of a
  daemon's pushes, queues the rest, and joins a transfer already running for
  the same key. Before a push, the client asks the registry whether it holds
  a layer and skips it when it does.
- [Git LFS's batch API](https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md):
  the client lists objects by hash and size, and the server gives upload
  actions only for those it lacks.
- [tus](https://tus.io/protocols/resumable-upload), and the IETF draft of
  resumable uploads that grew from it: `HEAD` answers the `Upload-Offset` the
  server holds, and the client sends from there.
- [Google Cloud Storage's resumable
  uploads](https://docs.cloud.google.com/storage/docs/resumable-uploads):
  chunks in multiples of 256 KiB, 8 MiB by default in its client libraries.
- Chrome sends a request the page needs to show first ahead of one it can
  defer.

Demi departs in that the queue is the device's, whoever asks, since the link
it guards is the device's. Its content store is the blob store it already
has, with its day of grace, not a cache of its own.

## Alternatives

- **Retry a broken upload and nothing else.** On a link that stalls under
  load, twenty retries stall it again.
- **One connection per upload instead of one HTTP/2 connection.** The spike
  broke 32 HTTP/1.1 connections as it broke 32 streams of one.
- **Adapt the number at once to the link's pace.** Git LFS tried scaling its
  default with the computer and went back to a fixed one; none of the tools
  above adapts it.
- **Keep the device's link alive by a longer silence.** Socket.IO counts a
  peer gone after 45 seconds, SignalR after 30, Kubernetes after 50; Demi's
  60 is among them. The silence was the runner's own uploads; queued, they
  leave the link room.

## Open questions

None.
