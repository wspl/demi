package process

import (
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"sort"
	"sync"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runnerwire"
)

// SpawnOptions supplies a child's executable, arguments, directory, environment and group ownership.
// Env replaces the inherited environment.
type SpawnOptions struct {
	Command      string
	Args         []string
	Cwd          string
	Env          map[string]string
	ProcessGroup bool
}

// ChildAttributes supplies a job's umask and resource limits. Unset values
// inherit the runner's, except open files retain the runner's startup limits.
// Windows ignores these Unix attributes.
type ChildAttributes struct {
	// Umask is absent when the process inherits the runner's mask.
	Umask *uint32
	// Limits lists each resource at most once.
	Limits []ResourceLimit
}

// ResourceLimit supplies the platform resource number and its soft and hard limits.
// Resource uses golang.org/x/sys/unix RLIMIT constants on Unix.
type ResourceLimit struct {
	Resource int
	Soft     uint64
	Hard     uint64
}

// SpawnFailure classifies a process that could not start.
type SpawnFailure struct {
	Kind    runnerwire.SpawnErrorKind
	Message string
	Cause   error
}

// Error returns the spawn failure's diagnostic.
func (e *SpawnFailure) Error() string { return e.Message }

// Unwrap preserves the underlying operating system error.
func (e *SpawnFailure) Unwrap() error { return e.Cause }

// OutputChunk is one read from a child's standard output or error.
type OutputChunk struct {
	Stream runnerwire.OutputStream
	Bytes  []byte
}

// Exit reports the child's status and any runner failure beside that status.
type Exit struct {
	Code   *int32
	Signal *string
	Error  *string
}

// Input is a chunk of process input. A nil Bytes slice means end of input;
// an empty non-nil slice is an empty chunk. Sending transfers ownership.
type Input struct{ Bytes []byte }

// Child owns a process, its input writer and output drains. Its owner must
// consume Output while waiting, or Cancel, and always Wait to join its work.
type Child struct {
	Input   chan<- Input
	Output  <-chan OutputChunk
	PID     uint32
	command *Command
	signals chan runnerwire.Signal
	cancel  context.CancelFunc
	done    chan struct{}
	exit    Exit
}

// Spawn starts a process with piped standard IO and startup child attributes.
// The context owns its lifetime; cancellation kills it and its owned group.
func Spawn(ctx context.Context, options SpawnOptions) (*Child, error) {
	owner, cancel := context.WithCancel(ctx)
	path, err := lookupExecutable(options.Command, options.Cwd, options.Env)
	if err != nil {
		cancel()
		return nil, classifyFailure(err, options)
	}
	cmd := &exec.Cmd{Path: path, Args: append([]string{options.Command}, options.Args...)}
	cmd.Dir = options.Cwd
	cmd.Env = make([]string, 0, len(options.Env))
	for key, value := range options.Env {
		cmd.Env = append(cmd.Env, key+"="+value)
	}
	sort.Strings(cmd.Env)
	input := make(chan Input, 4)
	output := make(chan OutputChunk, 4)
	child := &Child{Input: input, Output: output, cancel: cancel, done: make(chan struct{}), signals: make(chan runnerwire.Signal, 4)}
	// Command's IO adapters own these pipe endpoints and join each copy.
	stdin, writer := io.Pipe()
	cmd.Stdin = stdin
	outputCtx, stopOutput := context.WithCancel(context.Background())
	cmd.Stdout = &chunkWriter{ctx: outputCtx, cancel: stopOutput, output: output, stream: runnerwire.Stdout}
	cmd.Stderr = &chunkWriter{ctx: outputCtx, cancel: stopOutput, output: output, stream: runnerwire.Stderr}
	child.command = Wrap(cmd, options.ProcessGroup, ChildAttributes{})
	if err := child.command.Start(owner); err != nil {
		cancel()
		stopOutput()
		_ = stdin.Close() // In-memory pipes have no close failure.
		_ = writer.Close()
		return nil, classifyFailure(err, options)
	}
	child.PID = child.command.PID()
	written := make(chan struct{})
	var writeErr error
	go func() {
		defer close(written)
		defer func() { _ = writer.Close() }() // EOF closes the in-memory input pipe.
		for {
			select {
			case <-owner.Done():
				return
			case part, ok := <-input:
				if !ok || part.Bytes == nil {
					return
				}
				if _, writeErr = writer.Write(part.Bytes); writeErr != nil {
					return
				}
			}
		}
	}()
	go func() {
		defer close(child.done)
		defer cancel()
		defer stopOutput()
		var signalFailure error
	awaitExit:
		for {
			select {
			case <-child.command.done:
				break awaitExit
			case signal := <-child.signals:
				if err := child.command.Signal(signal); err != nil {
					signalFailure = err
				}
			}
		}
		child.exit = child.command.exit
		if signalFailure != nil {
			message := signalFailure.Error()
			child.exit.Error = &message
		}
		_ = stdin.Close() // Unblock a writer after the child no longer accepts input.
		_ = writer.Close()
		cancel()
		<-written
		if writeErr != nil && !errors.Is(writeErr, io.ErrClosedPipe) && !errors.Is(writeErr, syscall.EPIPE) {
			message := writeErr.Error()
			child.exit.Error = &message
		}
		close(output)
	}()
	return child, nil
}

// chunkWriter forwards child output with the same four-chunk backpressure as Rust.
type chunkWriter struct {
	cancel context.CancelFunc
	ctx    context.Context
	output chan<- OutputChunk
	stream runnerwire.OutputStream
}

func (w *chunkWriter) Close() error { w.cancel(); return nil }

func (w *chunkWriter) Write(b []byte) (int, error) {
	n := len(b)
	for len(b) > 0 {
		size := min(len(b), 64*1024)
		chunk := append([]byte(nil), b[:size]...)
		select {
		case <-w.ctx.Done():
			return n - len(b), w.ctx.Err()
		case w.output <- OutputChunk{Stream: w.stream, Bytes: chunk}:
		}
		b = b[size:]
	}
	return n, nil
}

// Signal queues a signal, or reports that the process has exited.
func (c *Child) Signal(ctx context.Context, signal runnerwire.Signal) error {
	select {
	case <-c.done:
		return &operationFailure{message: "process has exited", cause: io.ErrClosedPipe}
	default:
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	select {
	case <-c.done:
		return &operationFailure{message: "process has exited", cause: io.ErrClosedPipe}
	case <-ctx.Done():
		return ctx.Err()
	case c.signals <- signal:
		return nil
	}
}

// IsCancelled reports whether cancellation was requested.
func (c *Child) IsCancelled() bool { return c.command.cancelled() }

// Cancel requests termination. Wait must still join the child and its IO.
func (c *Child) Cancel() { c.command.requestCancel(); c.cancel() }

// Wait returns the cached exit after reaping and draining. Cancellation cancels
// the child and still joins it before returning.
func (c *Child) Wait(ctx context.Context) Exit {
	select {
	case <-c.done:
	case <-ctx.Done():
		c.Cancel()
		<-c.done
	}
	return c.exit
}

// Command owns a caller-configured exec.Cmd and its process group. The caller
// configures IO before Wrap and owns any pipes it creates. After Start succeeds,
// it must call Wait, including after Kill or cancellation. Blocking non-file
// IO must have a Close that unblocks it; cancellation closes those adapters,
// and leader exit closes the input adapter. Supplied *os.File values stay
// owned by the caller.
type Command struct {
	template      *exec.Cmd
	attributes    ChildAttributes
	group         bool
	cmd           *exec.Cmd
	platform      platformGroup
	done          chan struct{}
	ctx           context.Context
	cancel        context.CancelFunc
	exit          Exit
	cancelledFlag bool
	mu            sync.Mutex
}

// Wrap gives a command its child attributes and optional process group (a
// Windows Job Object). The caller must not start or wait on cmd directly.
func Wrap(cmd *exec.Cmd, group bool, attributes ChildAttributes) *Command {
	attributes.Limits = append([]ResourceLimit(nil), attributes.Limits...)
	if attributes.Umask != nil {
		value := *attributes.Umask
		attributes.Umask = &value
	}
	return &Command{template: cmd, group: group, attributes: attributes}
}

// Start starts the configured command, retrying transient descriptor and busy
// executable errors. The context owns the child's lifetime through Wait.
func (c *Command) Start(ctx context.Context) error {
	c.ctx, c.cancel = context.WithCancel(ctx)
	ctx = c.ctx
	cmd := c.template
	// Copy the public command configuration; exec.Cmd itself is never reused
	// after a failed Start, which otherwise closes its internally-owned pipes.
	prepared := &exec.Cmd{Path: cmd.Path, Args: append([]string(nil), cmd.Args...), Env: cmd.Env, Dir: cmd.Dir,
		Stdin: cmd.Stdin, Stdout: cmd.Stdout, Stderr: cmd.Stderr, ExtraFiles: cmd.ExtraFiles, SysProcAttr: cmd.SysProcAttr, Err: cmd.Err}
	streams, err := prepareStreams(ctx, prepared)
	if err != nil {
		c.cancel()
		return err
	}
	started, err := Start(ctx, func() (*exec.Cmd, error) {
		return startPlatform(ctx, prepared, c.group, c.attributes, &c.platform)
	})
	if err != nil {
		streams.close()
		c.cancel()
		return err
	}
	streams.cancel = c.cancel
	c.cmd = started
	c.done = make(chan struct{})
	streams.start()
	go c.own(streams)
	return nil
}

// own reaps the leader before draining output, so descendants cannot keep a
// group-owned command's pipes open after its leader has exited.
func (c *Command) own(streams *commandStreams) {
	defer close(c.done)
	defer c.cancel()
	interrupted := make(chan struct{})
	stop := context.AfterFunc(c.ctx, func() {
		c.mu.Lock()
		c.cancelledFlag = true
		c.mu.Unlock()
		killErr := c.Kill()
		streams.interrupt()
		streams.record(killErr)
		close(interrupted)
	})
	err := c.cmd.Wait()
	streams.stopInput()
	if c.group {
		streams.record(c.Kill())
	}
	streams.wait(c.ctx)
	if !stop() {
		<-interrupted
	}
	c.exit = commandExit(c.cmd.ProcessState, err)
	if failure := streams.failure(); failure != nil {
		message := failure.Error()
		c.exit.Error = &message
	}
	c.platform.close()
}

// Wait reaps the command and joins its owned work. Context cancellation kills
// it and its group before joining. The result includes nonzero exit status.
func (c *Command) Wait(ctx context.Context) Exit {
	select {
	case <-c.done:
	case <-ctx.Done():
		c.requestCancel() // The owner kills, interrupts IO and joins all work.
		<-c.done
	}
	return c.exit
}

// Kill terminates the command and its owned group; an already gone group succeeds.
func (c *Command) Kill() error {
	if c.cmd == nil {
		return os.ErrProcessDone
	}
	select {
	case <-c.done:
		return nil
	default:
	}
	return c.platform.kill(c.cmd.Process, c.group)
}

// Signal sends a platform-supported signal to the command and its owned group.
func (c *Command) Signal(signal runnerwire.Signal) error {
	if c.cmd == nil {
		return os.ErrProcessDone
	}
	select {
	case <-c.done:
		return nil
	default:
	}
	return c.platform.signal(c.cmd.Process, c.group, signal)
}

// PID returns the process ID after a successful Start.
func (c *Command) PID() uint32 { return uint32(c.cmd.Process.Pid) }
func (c *Command) requestCancel() {
	c.mu.Lock()
	c.cancelledFlag = true
	c.mu.Unlock()
	c.cancel()
}
func (c *Command) cancelled() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.cancelledFlag
}

// Start retries a single spawn attempt on descriptor exhaustion and briefly
// on a busy executable. Both async and blocking Rust callers use this function.
func Start[T any](ctx context.Context, attempt func() (T, error)) (T, error) {
	var backoff cmdsdk.Backoff
	var busy time.Duration
	for {
		if err := ctx.Err(); err != nil {
			var zero T
			return zero, err
		}
		value, err := attempt()
		if err == nil {
			return value, nil
		}
		exhausted := cmdsdk.Exhausted(err)
		if !exhausted && (!errors.Is(err, syscall.ETXTBSY) || busy >= time.Second) {
			return value, err
		}
		pause := backoff.Pause()
		if !exhausted {
			busy += pause
		}
		timer := time.NewTimer(pause)
		select {
		case <-ctx.Done():
			timer.Stop()
			return value, ctx.Err()
		case <-timer.C:
		}
	}
}

// classifyFailure preserves the operating system cause beside the wire category.
func classifyFailure(err error, options SpawnOptions) *SpawnFailure {
	kind := runnerwire.SpawnErrorKindOther
	info, statErr := os.Stat(options.Cwd)
	switch {
	case statErr != nil || !info.IsDir():
		kind = runnerwire.SpawnErrorKindCwdUnusable
	case errors.Is(err, os.ErrNotExist):
		kind = runnerwire.SpawnErrorKindExecutableNotFound
	case errors.Is(err, syscall.EISDIR):
		kind = runnerwire.SpawnErrorKindIsDirectory
	case errors.Is(err, os.ErrPermission):
		kind = runnerwire.SpawnErrorKindPermissionDenied
	}
	return &SpawnFailure{Kind: kind, Message: err.Error(), Cause: err}
}

// commandExit retains a known exit status even when runner IO also failed.
func commandExit(state *os.ProcessState, err error) Exit {
	var result Exit
	if state != nil {
		if code := state.ExitCode(); code >= 0 {
			value := int32(code)
			result.Code = &value
		} else {
			result.Signal = exitSignal(state)
		}
	}
	var exitError *exec.ExitError
	if err != nil && !errors.As(err, &exitError) {
		message := err.Error()
		result.Error = &message
	}
	return result
}
