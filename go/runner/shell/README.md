# Shell jobs

`Run(ctx, script, Options)` creates one interpreter and owns its input file,
processes, background tasks and redirections until completion. Supply the full
job environment and a context-aware `Sink`; output is serialized. A nonzero
shell status is a `Result.Code`; cancellation returns `Result.Signal`.

The executable must call `shell.ExecHelper()` at the beginning of `main`, before
runner initialization. Ordinary invocations return immediately. Private child
invocations apply umask and limits and exec the target on Unix. On Windows the
helper joins the parent's Job Object before launching the target. Shell tests
call this entry point from `TestMain`.

G5c supplies command-service leases, RPC invocation/cancellation, command-context
lifetime and hint ownership through `Commands`. It can also call
`Commands.Dispatch` for the local-client transport. The trusted invocation
context is separate from shell environment variables. `ReportEdits` accepts
G5c's histogram counter; service-failure enrichment also belongs to G5c.

The vendored interpreter is a separate Go module; its regression suite must be
run separately. See `../../third_party/mvdan-sh/README.md` for the patch list.

All process starts, including helper target execs, use `runner/process.Start`.
The caller supplies a start attempt that releases its resources on failure;
reaping a successful child is separate and is never retried. Every attempt
constructs a fresh `exec.Cmd`, including the native limit probe and Windows
helper: Go rejects a second `Start` even after the first failed. The shared policy
uses `commandservice.Backoff`: descriptor exhaustion waits until canceled;
ETXTBSY consumes a cumulative one-second wait budget. Interpreter internal
allocations use the same descriptor policy through `RetryHandler`.

The first open-file limit query caches the inherited soft and hard limits from
`/bin/sh -c 'ulimit -Sn; ulimit -Hn'`. Go restores the original limits in this
native child; another Go helper would raise them again before its code runs.
A failed probe logs its reason through `slog` and uses the runner's limits.
G5c routes the standard logger to the Host log.
