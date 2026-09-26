// Package shell runs shell jobs inside the runner process
// (docs/demi-next/runner.md § Shell jobs).
//
// A job is a fresh login shell on Demi's fork of mvdan.cc/sh
// (third_party/mvdan-sh). It owns its working directory, environment, umask,
// standard streams and all of its concurrent work: every interpreter task,
// utility, pipe, open file and external process. Many jobs share the process,
// so a job never reads or changes process-global state; each command gets
// what a process would take from the operating system from its job.
//
// Commands run in three ways. A declared root goes to the job's Dispatcher. A
// standard utility is a Go function from the utility registry that runs on
// its own goroutine through a toolctx.Invocation. Anything else is an external
// program started in its own process group. In-process commands exchange
// bytes through memory; a pipe becomes an OS pipe only when an external
// program takes one of its ends.
//
// Every write-capable open, write, whole-file write and temporary-file
// publication by the shell or a utility goes through the job's EditRecorder
// (docs/demi-next/edit-tracking.md).
package shell

import (
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os/user"
	"strings"
	"sync"

	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"

	"github.com/wspl/demi/internal/toolctx"
)

// Config is what every job of one runner shares.
type Config struct {
	// Utilities are the standard utilities by name, normally tools.Utilities.
	Utilities map[string]toolctx.Utility
	// SystemProfile is the login profile every job reads first, normally
	// /etc/profile. Empty means none.
	SystemProfile string
	// Umask is every job's initial file mode creation mask, normally 0o022.
	Umask fs.FileMode
}

// Shell starts jobs.
type Shell struct {
	config Config
}

// New returns a Shell that starts jobs with config.
func New(config Config) *Shell {
	return &Shell{config: config}
}

// Spec describes one job.
type Spec struct {
	// Script is the shell program the job runs.
	Script string
	// Dir is the absolute working directory the script starts in.
	Dir string
	// Env is the job's environment as "name=value" entries. The login
	// profiles cannot replace the runner-owned variables in it: those named
	// DEMI_*, TMPDIR and TEMP.
	Env []string
	// Commands is the job's command context, or nil when it has none.
	Commands *Commands
	// Edits records the files the job writes, or is nil to record nothing.
	Edits EditRecorder
}

// Commands is the part of a job's command context that the shell uses.
type Commands struct {
	// Dispatcher runs the declared roots.
	Dispatcher Dispatcher
	// AliasDir is the directory of the command aliases. It stays first in
	// PATH after the login profiles.
	AliasDir string
}

// Dispatcher runs declared commands for one job. It is bound to the job's
// execution context and pinned manifest (docs/demi-next/commands.md).
type Dispatcher interface {
	// Declared reports whether name is a declared root.
	Declared(name string) bool
	// Dispatch runs a declared root: cmd.Args[0] is the root and the rest are
	// its raw arguments. Dispatch returns the exit status once the command
	// completed and released its streams, or an error when it could not run
	// it. It stops when ctx ends.
	Dispatch(ctx context.Context, cmd toolctx.Command) (int, error)
}

// EditRecorder records the files a job creates or modifies
// (docs/demi-next/edit-tracking.md § Recording actual writes).
type EditRecorder interface {
	// Record runs op, which may change the contents of the file at path, and
	// records the file's contents before and after it. path is absolute and
	// names a regular file or nothing: the job never records pipes, devices
	// or directories. Record runs op even when recording fails.
	Record(path string, op func())
}

// Status is how a job ended.
type Status int

const (
	// Exited means the script ran to its end; Result.Code is its status.
	Exited Status = iota + 1
	// Cancelled means the job was cancelled; Result.Signal names why.
	Cancelled
	// Failed means the shell itself failed; Result.Err says why.
	Failed
)

// Result is what a job reports once all of its work has finished.
type Result struct {
	Status Status
	// Code is the exit status of the script, when Status is Exited.
	Code int
	// Signal is the signal that requested the cancellation, or SIGKILL when
	// none did, when Status is Cancelled.
	Signal string
	// Dir is the working directory the script ended in, when Status is
	// Exited.
	Dir string
	// Err is the failure, when Status is Failed.
	Err error
}

// Job is one running shell job.
type Job struct {
	shell *Shell
	spec  Spec

	ctx    context.Context
	cancel context.CancelFunc

	// stdin, stdout and stderr are the job's ends of its standard streams.
	stdin  *pipeReader
	stdout *pipeWriter
	stderr *pipeWriter
	// Their other ends belong to the caller.
	input  *pipeWriter
	output *pipeReader
	errors *pipeReader

	// tasks counts the job's goroutines: interpreter tasks, utilities,
	// stream copies and process reapers.
	tasks sync.WaitGroup

	mu sync.Mutex
	// signal is the first cancellation request, or "" before one.
	signal string
	// failure is the first panic of a task.
	failure error
	// resources are the pipes and files the job has open. Cancellation closes
	// them, which interrupts every read and write waiting on them.
	resources map[io.Closer]struct{}

	done   chan struct{}
	result Result
}

// Start starts a job. It reports a problem with spec, such as a working
// directory that does not exist, as an error; everything that happens later
// ends up in the job's Result.
func (s *Shell) Start(spec Spec) (*Job, error) {
	ctx, cancel := context.WithCancel(context.Background())
	j := &Job{
		shell:     s,
		spec:      spec,
		ctx:       ctx,
		cancel:    cancel,
		resources: make(map[io.Closer]struct{}),
		done:      make(chan struct{}),
	}
	j.stdin, j.input = j.newPipe()
	j.output, j.stdout = j.newPipe()
	j.errors, j.stderr = j.newPipe()
	runner, err := interp.New(
		interp.Env(expand.ListEnviron(j.environment()...)),
		interp.Dir(spec.Dir),
		interp.StdIO(j.stdin, j.stdout, j.stderr),
		interp.Umask(s.config.Umask),
		interp.Tasks(j.goTask),
		interp.PipeHandler(j.pipeHandler),
		interp.OpenHandler(j.openHandler),
		interp.ExecHandlers(j.execMiddleware),
	)
	if err != nil {
		j.release()
		cancel()
		return nil, err
	}
	go j.run(runner)
	return j, nil
}

// environment is the job's environment with HOME set, which Bash sets from
// the user database when the environment lacks it.
func (j *Job) environment() []string {
	env := j.spec.Env
	if _, ok := newEnviron(env).Get("HOME"); ok {
		return env
	}
	account, err := user.Current()
	if err != nil {
		// Without HOME, the job reads no user profile; that is all it loses.
		return env
	}
	return append(env[:len(env):len(env)], "HOME="+account.HomeDir)
}

// Stdin is the job's standard input. Each write waits until commands have
// read all of it, so input that no command asks for stays with the caller.
// Close it to give the job the end of file.
func (j *Job) Stdin() io.WriteCloser { return j.input }

// Stdout is the job's standard output. It reaches the end of file once the
// job is done. Nothing the job writes is buffered: a caller that stops
// reading holds the job's writers back until the job is cancelled.
func (j *Job) Stdout() io.Reader { return j.output }

// Stderr is the job's standard error, like Stdout.
func (j *Job) Stderr() io.Reader { return j.errors }

// Done is closed once the job and all of its work have finished.
func (j *Job) Done() <-chan struct{} { return j.done }

// Result is how the job ended. It is valid once Done is closed.
func (j *Job) Result() Result {
	<-j.done
	return j.result
}

// cancellationSignals are the signals a job accepts as a cancellation request.
var cancellationSignals = map[string]bool{
	"SIGINT":  true,
	"SIGTERM": true,
	"SIGKILL": true,
	"SIGHUP":  true,
	"SIGQUIT": true,
}

// Signal cancels the job on behalf of signal, one of SIGINT, SIGTERM,
// SIGKILL, SIGHUP and SIGQUIT, which the Result then reports. A later request
// does not replace the first.
func (j *Job) Signal(signal string) error {
	if !cancellationSignals[signal] {
		return fmt.Errorf("unsupported shell job signal %q", signal)
	}
	j.stop(signal)
	return nil
}

// Cancel cancels the job without a signal; the Result reports SIGKILL.
func (j *Job) Cancel() {
	j.stop("SIGKILL")
}

// stop cancels the job and closes everything its work may wait on.
// External processes are killed by their reapers.
func (j *Job) stop(signal string) {
	j.mu.Lock()
	select {
	case <-j.done:
		j.mu.Unlock()
		return
	default:
	}
	if j.signal == "" {
		j.signal = signal
	}
	j.cancel()
	resources := make([]io.Closer, 0, len(j.resources))
	for resource := range j.resources {
		resources = append(resources, resource)
	}
	clear(j.resources)
	j.mu.Unlock()
	for _, resource := range resources {
		// Closing only interrupts the work; its error changes nothing.
		resource.Close()
	}
}

// track adds a resource that cancellation closes, and returns false, having
// closed it, when the job is already cancelled.
func (j *Job) track(resource io.Closer) bool {
	j.mu.Lock()
	defer j.mu.Unlock()
	if j.ctx.Err() != nil {
		resource.Close()
		return false
	}
	j.resources[resource] = struct{}{}
	return true
}

// untrack removes a resource that its user closed.
func (j *Job) untrack(resource io.Closer) {
	j.mu.Lock()
	defer j.mu.Unlock()
	delete(j.resources, resource)
}

// release closes the resources that remain once all work has finished, such
// as files that "exec 3>file" kept open.
func (j *Job) release() {
	j.mu.Lock()
	resources := make([]io.Closer, 0, len(j.resources))
	for resource := range j.resources {
		resources = append(resources, resource)
	}
	clear(j.resources)
	j.mu.Unlock()
	for _, resource := range resources {
		// Nothing uses them any more; a close error has no one to tell.
		resource.Close()
	}
}

// goTask runs task on a goroutine that the job waits for. A panic fails the
// job instead of the runner.
func (j *Job) goTask(task func()) {
	j.tasks.Add(1)
	go func() {
		defer j.tasks.Done()
		defer j.recoverTask()
		task()
	}()
}

// recoverTask turns a panic of the calling task into the job's failure and
// cancels the job's remaining work.
func (j *Job) recoverTask() {
	value := recover()
	if value == nil {
		return
	}
	j.mu.Lock()
	if j.failure == nil {
		j.failure = fmt.Errorf("shell task panicked: %v", value)
	}
	j.mu.Unlock()
	j.stop("SIGKILL")
}

// newPipe creates a pipe that cancellation closes.
func (j *Job) newPipe() (*pipeReader, *pipeWriter) {
	r, w := newPipe()
	closer := pipeCloser{r.p}
	r.p.closed = func() { j.untrack(closer) }
	j.track(closer)
	return r, w
}

// pipeCloser closes both ends of a pipe.
type pipeCloser struct{ p *pipe }

func (c pipeCloser) Close() error {
	return errors.Join(c.p.closeReader(), c.p.closeWriter())
}

// pipeHandler implements interp.PipeHandlerFunc with in-memory pipes.
func (j *Job) pipeHandler(ctx context.Context) (io.ReadCloser, io.WriteCloser, error) {
	r, w := j.newPipe()
	return r, w, nil
}

// run runs the login profiles and the script, waits for all of the job's
// work, and reports the result.
func (j *Job) run(runner *interp.Runner) {
	var result Result
	func() {
		// The script runs as a task, so its panic fails the job too.
		j.tasks.Add(1)
		defer j.tasks.Done()
		defer j.recoverTask()
		result = j.runScript(runner)
	}()
	j.tasks.Wait()
	// Everything that wrote the standard streams has finished.
	j.stdout.Close()
	j.stderr.Close()
	j.stdin.Close()
	j.mu.Lock()
	switch {
	case j.signal != "":
		result = Result{Status: Cancelled, Signal: j.signal}
	case j.failure != nil:
		result = Result{Status: Failed, Err: j.failure}
	}
	j.result = result
	close(j.done)
	j.mu.Unlock()
	j.release()
	j.cancel()
}

// runScript runs the login profiles, restores the job's own context, and
// runs the script.
func (j *Job) runScript(runner *interp.Runner) Result {
	script, err := syntax.NewParser().Parse(strings.NewReader(j.spec.Script), "")
	if err != nil {
		// Like Bash, report a syntax error on standard error with status 2.
		fmt.Fprintln(j.stderr, err)
		return Result{Status: Exited, Code: 2, Dir: j.spec.Dir}
	}
	if code, exited := j.login(runner); exited {
		return Result{Status: Exited, Code: code, Dir: runner.Dir}
	}
	return exitResult(runner.Run(j.ctx, script), runner.Dir)
}

// exitResult converts what interp.Runner.Run returned.
func exitResult(err error, dir string) Result {
	var status interp.ExitStatus
	switch {
	case err == nil:
		return Result{Status: Exited, Dir: dir}
	case errors.As(err, &status):
		return Result{Status: Exited, Code: int(status), Dir: dir}
	default:
		return Result{Status: Failed, Err: err}
	}
}
