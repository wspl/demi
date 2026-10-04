# go-git version evidence

The r-host API checkpoint selects **v5.19.2**, following the stable-v5
decision in [Go migration](../../../../../docs/delivery/go-migration.md#tech-lead-decisions).
It passes the status fixtures and shallow HTTPS clone/fetch. Its failure to
shallow-clone a local repository without Git does not affect a product path:
the runner never clones, and skill sources accept GitHub `owner/repo` or HTTPS
repositories. The earlier v6 selection gave that out-of-scope probe weight it
should not have had.

Tests needing a repository to fetch must serve it over HTTP from the test,
using go-git's server or the smart HTTP protocol, never a local file transport.
The historical local-transport probe remains below solely as recorded evidence.
No public Host API exposes go-git types, so switching versions requires no API
signature changes. The retained patch documents v5 implementation adaptations.

[evidence.txt](evidence.txt) records the commands and outcomes. The two
comparison files record v5 against the existing Demi and Kubernetes fixtures.
This is version-selection evidence, not an implemented Host or proof of the
remaining Git edge cases listed in sp07's report.

To reproduce on macOS, copy the Go files and `verify.py` from the sp07 `spike/`
directory into a disposable directory. Apply [v5.patch](v5.patch) with
`patch -p1`, and copy `v5.mod` and `v5.sum` to `go.mod` and `go.sum`.
Run all Go commands with `CGO_ENABLED=0`:

```sh
go test -v ./...
go build -o sp07 .
mkdir evidence
python3 verify.py check /absolute/path/to/sp07/spike/tmp/demi demi
python3 verify.py check /absolute/path/to/sp07/spike/tmp/kubernetes kubernetes
```

`verify.py` uses Git as a read-only oracle with `GIT_OPTIONAL_LOCKS=0`;
it writes evidence only in the disposable directory. The patch changes
versioned imports, v5's filesystem field, SHA-1 hasher, tag constants and clone
signature, reads `core.filemode` from raw config, closes storage through its
`io.Closer` capability, and selects v5's in-process file server. It does not
change status, rename or line-count algorithms. The `.git` endpoint is required
by that server's loader; it still fails on the shallow capability.

The network commands in `evidence.txt` are manual spike checks, not automated
tests. Reproduce v6 by copying the original sp07 source and module files without
the patch. Temporary sources, binaries and clone destinations were removed
after recording these results. Native FSEvents and production Host behavior
remain for the implementation checkpoint.
