---
'@demicodes/shell': patch
---

Bound every in-memory ingestion path of the shell environment. The execution model buffers whole command outputs (in several copies) and portable commands load files whole, so one stray `rg`/`cat` over a large tree could balloon the embedding process by tens of gigabytes until the kernel OOM-killed it.

- Foreground host processes now answer to `maxCaptureBytes` (default 64 MiB): output beyond the ceiling kills the process and fails the command with an explicit error naming the limit.
- Background jobs retain only the most recent output within the view budget and report how much they dropped.
- In-shell file reads (`HostBackedFileSystem`) refuse files over `maxFileReadBytes` (default 64 MiB) with an explicit error pointing at real-process routes for large files.
- The interpreter output ceiling now aligns with the capture limit instead of a 1 GiB blanket raise.
