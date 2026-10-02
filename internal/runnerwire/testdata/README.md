The MessagePack frames, kept records, and manifest.json are copied verbatim
from crates/runner-protocol/tests/fixtures. The 115 directional frames await
the runner wire's generated codec; records_test.go currently exercises only
the kept-output fixture.

The probes below reproduce upstream generation blockers without changing
another package. Run each with CGO_ENABLED=0 GOFLAGS=-mod=readonly:

    go run ./tools/contractgen ./internal/runnerwire/testdata/probes/opaque
    go run ./tools/contractgen ./internal/runnerwire/testdata/probes/timestamp
    go run ./tools/contractgen ./internal/runnerwire/testdata/probes/imported
    go test ./internal/runnerwire/testdata/probes/imported

The last two commands demonstrate that generating a consumer does not emit
MessagePack methods into its imported contract package.
