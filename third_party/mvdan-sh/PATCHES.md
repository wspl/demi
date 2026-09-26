# Demi's changes to mvdan.cc/sh

Upstream: https://github.com/mvdan/sh, tag v3.14.1, BSD-3-Clause (`LICENSE`).
Maintained by `internal/shell`, which runs Demi's shell jobs on this
interpreter (`docs/demi-next/runner.md` § Shell jobs). The root `go.mod`
points `mvdan.cc/sh/v3` here with a `replace` directive.

Every change from the upstream release, with its reason:

## Build with the Go version Demi pins

Upstream v3.14 requires Go 1.26; Demi's `go.mod` pins Go 1.24.

- `go.mod`: `go 1.24`, and the newest `golang.org/x/sys` and `golang.org/x/term`
  releases that support Go 1.24. The `tool` directive and the dependencies of
  the removed programs are gone.
- `internal/astype.go` adds `AsType`, the Go 1.26 `errors.AsType`; `interp`
  and its tests call it instead.
- `interp/runner.go`: a pipeline starts its left side with `WaitGroup.Add`
  and `go` instead of the Go 1.25 `WaitGroup.Go`.
- `syntax/walk.go` and `syntax/parser_test.go` use `Value.Interface` and
  `Value.Field` instead of the Go 1.25 `reflect.TypeAssert` and `Value.Fields`.
- `cmd/` (shfmt, gosh) and `syntax/typedjson` are removed: Demi uses neither,
  and they need newer Go or further dependencies.

## Embed the interpreter in a process that runs many shells

Demi runs every shell job inside the runner process, so the interpreter must
not rely on process-wide state and must let its embedder account for all of
its work (`docs/demi-next/runner.md` § Shell jobs).

- `Tasks` (`interp/api.go`) lets the embedder start the interpreter's
  goroutines: background commands, pipeline stages, process substitutions and
  here-document writers. `Run` does not wait for background work; a job counts
  these tasks to know when everything has finished.
- `PipeHandler` and `PipeHandlerFunc` (`interp/api.go`, `interp/handler.go`)
  create the pipes of pipelines, here-documents and here-strings, so the
  embedder can connect in-process commands through memory. The default is
  `os.Pipe`. Standard input is any `io.Reader` and is no longer copied into an
  `os.Pipe` by `StdIO` (`interp/stdin_os.go`, `interp/stdin_js.go`); the read
  builtin cancels a blocked read through `SetReadDeadline` when the reader has
  it. The default exec handler lets `os/exec` copy such a reader, which
  `TestRunnerNonFileStdin` now expects.
- Process substitutions use a pipe named `/dev/fd/N`, like Bash, instead of a
  named pipe in `$TMPDIR` (`interp/runner.go`). A named pipe blocks its
  substitution until someone opens it, so `echo <(true)` never finished, and
  its directory came from the process environment. The shell keeps its end
  until the statement that expanded it finishes, lists it in
  `HandlerContext.Descriptors`, and the default exec handler passes it to the
  program as `ExtraFiles`. `Runner.tempDir` and `mkfifo` are gone.
- A path that names a descriptor, `/dev/stdin`, `/dev/stdout`, `/dev/stderr`
  or `/dev/fd/N`, opens the shell's own descriptor instead of the process's
  (`Runner.open`, `DescriptorPath`). In a shared process, `echo x >/dev/stderr`
  would otherwise write to the runner's standard error.
- `ExecEnv` (`interp/vars.go`) is exported, so an exec handler builds a
  program's environment the way the default one does.

## Shell features the jobs need

- File descriptors above 2 (`Runner.fds`, `Descriptor`,
  `HandlerContext.Descriptors`): `N>file`, `N>>file`, `N<file`, `N<>file`,
  `N>&M`, `N<&M` and `N>&-` for any N, kept by `exec` and closed by
  `exec N>&-` once no descriptor refers to a file that this shell opened.
  `<>` and `>|` are supported. Redirections create files with mode 0666, which
  the open handler reduces by the umask, as Bash does; upstream used 0644. The
  redirection tests that expected "unsupported" errors now expect Bash's
  results.
- `umask [-p] [-S] [mode]` with octal and symbolic modes, the `Umask` option
  and `HandlerContext.Umask` (`interp/builtin.go`). The interpreter keeps the
  mask per shell and subshell; handlers apply it, since the process umask is
  shared by all jobs.
- `kill` (`interp/builtin.go`): a background command named by `$!` (`g1` and
  so on) is stopped by cancelling its context, and `wait` then reports
  128 plus the signal number. Other process IDs, and `kill -l`, go to the kill
  program through the exec handler.
- A builtin whose standard output write fails with `EPIPE` makes the shell
  exit with status 141, as SIGPIPE makes Bash exit (`Runner.call`), so
  `while :; do echo y; done | head -n1` ends.

New cases in `interp/interp_test.go` cover the descriptors, descriptor paths,
process substitution paths, SIGPIPE, umask and kill.

The remaining upstream test failures, `cd` into directories without search
permission (`TestRunnerRun`, "mkdir a; chmod 0000 a; cd a" and the like),
happen only as root, where access(2) allows everything; they fail on upstream
too.
