# Binary stream boundary

Commands produce byte streams; the model boundary decides how the final
stream is presented. There is no attachment side-channel: the `CommandAsset`
/ `io.asset()` mechanism is removed, and with it every `supportedAssetTypes`
thread through exec/session/ctx.

## Verified transport contract (just-bash)

- Fork pipes carry bytes packed as latin1 strings (`ByteString`); commands
  declare `stdoutKind: 'text' | 'bytes'` on `ExecResult`.
- The `hostSpawn` callback receives `stdin: string` in the **same latin1
  convention** (the identical value is tagged `unsafeBytesFromLatin1` when
  handed to registered commands), and its returned `ExecResult` participates
  in pipeline continuation via `stdoutKind`.
- The top-level `executeScript` result reaches `BashEnvironment.collectExited`
  with `stdoutKind` intact — that is the single boundary where stream shape
  is decided.

## Byte fidelity fixes

- `hostSpawn` stdin: write latin1 bytes to the real process (the previous
  `encodeUtf8` mojibaked every byte > 0x7F).
- `hostSpawn` result: return raw output bytes as latin1 + `stdoutKind:
  'bytes'` so real-process output (ffmpeg) pipes onward losslessly. The
  streaming observation channel keeps its lossy UTF-8 text render; file
  redirect sinks already preserve bytes.
- Registered-command adapter: `CommandIO.stdout(Uint8Array)` returns latin1
  + `stdoutKind: 'bytes'` to the pipe; `CommandStdin` carries `bytes`
  alongside `text`.
- `demi read` is `cat`: it emits raw file bytes for every file type. Media
  summaries and model-capability errors move to the boundary.

## Boundary decision tree (agent layer, where the model is known)

For the final stream of an exited exec:

1. `stdoutKind: 'text'` → tool-result text (unchanged).
2. bytes that strictly decode as UTF-8 → text (covers real-process text
   output).
3. bytes whose magic matches the closed model-media set (png, jpeg, gif,
   webp, mp4/m4v, mov, webm) **and** the model accepts that media type and
   the stream was not truncated → native image/video block, with a one-line
   text summary in the tool result.
4. anything else → placeholder text stating byte count, sniffed type when
   known, and the reason no media was attached (truncated by the output
   limit / model does not accept the type / unknown binary). Raw bytes never
   enter the transcript; they stay addressable at
   `/@/commands/<commandId>/stdout.bin` for further shell processing.

The shell layer only detects and reports (`binaryStdout: { data, mediaType,
truncated }` on exited snapshots plus a placeholder text render); model
capability gating lives in the agent layer where the tool result is built.
Sniffing a closed set by magic bytes is deterministic; there is no
content-type guessing beyond it.

Accepted edge: a mixed stream (`demi read a.png; echo done`) matches magic at
offset 0 and ships with trailing bytes — decoders tolerate trailing data, and
the fix is to not mix streams.

## Bridge

The UDS `/run` result carries `stdoutEncoding: 'base64'` when the final
stream is binary; the shim decodes and writes raw bytes to its OS stdout, so
`demi read a.png | ffmpeg -i - …` works in external shells too. A truncated
binary stream is reported on stderr alongside the capped payload.
