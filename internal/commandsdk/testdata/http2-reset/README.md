# HTTP/2 reset returns receive capacity twice

This standalone standard-library regression demonstrates the connection loss
behind `TestAbandoningBurstKeepsConnection`. It imports no Demi code. It stays
in `testdata` so it can be supplied independently to the Go maintainers; run it
explicitly from the repository root:

```sh
CGO_ENABLED=0 go test -race ./internal/commandsdk/testdata/http2-reset -count=50 -timeout=30s
```

On Linux, use `CGO_ENABLED=1` and add `-tags netgo,osusergo` for the race runtime.
The observed toolchain is Go 1.27.1, darwin/arm64. The test expects a second
request on the same connection to succeed. It instead reports
`http2: client conn not usable` or a connection error, with a server panic:

```text
flow control update exceeds maximum window size
net/http/internal/http2.(*inflow).add
net/http/internal/http2.(*serverConn).sendWindowUpdate
net/http/internal/http2.(*serverConn).noteBodyRead
```

The handler reads one byte of a 4096-byte body, flushes response headers, waits
for the client's cancellation, then reads the remaining buffered bytes inside
the handler. There are no concurrent body reads and no use after handler return.
An interrupted read is allowed to return buffered bytes; a reset must not break
unrelated requests. Cancellation after the buffered read starts has the same
accounting problem, so checking the context before each SDK read cannot fix it.

In Go 1.27.1's `src/net/http/internal/http2/server.go`, `closeStream` refunds
`p.Len()` to the connection and calls `p.CloseWithError(err)`. The latter still
allows reads of buffered bytes (`pipe.go`, `pipe.Read`). A later read reports
those bytes through `noteBodyRead`, which refunds them again, including for a
closed stream. At the configured maximum connection window, this immediately
violates `inflow.add`'s maximum-window check. A smaller window merely leaves
room for the incorrect credits to accumulate.

The narrow fix belongs in the transport: on reset, discard/close unread bytes
before measuring and refunding them, atomically with respect to reads. For
example, evaluate using `BreakWithError` before measuring `Len` in the stream
closure path: it stops subsequent reads and retains the unread count. Bytes
already taken by a reader must receive their credit exactly once through
`noteBodyRead`. This is a proposal for a reviewed Go fix, not a patch applied by
this work package. The safe deployment proposal is to retain the SDK's single
connection and full window on a corrected Go transport, validated against this
reproduction and the SDK cancellation suite. There is no demonstrated safe
SDK-only workaround that preserves prompt cancellation and the required window.

No toolchain files were changed, and no upstream issue was submitted. The SDK
burst test stays skipped, naming this reproduction, until the transport is
fixed; a passing run would not resolve this scheduling-dependent defect.
