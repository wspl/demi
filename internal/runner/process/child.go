package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

import (
	"context"
	"os/exec"

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
func (e *SpawnFailure) Error() string { panic("not written: r-process") }

// Unwrap preserves the underlying operating system error.
func (e *SpawnFailure) Unwrap() error { panic("not written: r-process") }

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
	Input  chan<- Input
	Output <-chan OutputChunk
	PID    uint32
}

// Spawn starts a process with piped standard IO and startup child attributes.
// The context owns its lifetime; cancellation kills it and its owned group.
func Spawn(ctx context.Context, options SpawnOptions) (*Child, error) {
	panic("not written: r-process")
}

// Signal queues a signal, or reports that the process has exited.
func (c *Child) Signal(ctx context.Context, signal runnerwire.Signal) error {
	panic("not written: r-process")
}

// IsCancelled reports whether cancellation was requested.
func (c *Child) IsCancelled() bool { panic("not written: r-process") }

// Cancel requests termination. Wait must still join the child and its IO.
func (c *Child) Cancel() { panic("not written: r-process") }

// Wait returns the cached exit after reaping and draining. Cancellation cancels
// the child and still joins it before returning.
func (c *Child) Wait(ctx context.Context) Exit { panic("not written: r-process") }

// Command owns a caller-configured exec.Cmd and its process group. The caller
// configures IO before Wrap and owns any pipes it creates. After Start succeeds,
// it must call Wait, including after Kill or cancellation.
type Command struct{}

// Wrap gives a command its child attributes and optional process group (a
// Windows Job Object). The caller must not start or wait on cmd directly.
func Wrap(cmd *exec.Cmd, group bool, attributes ChildAttributes) *Command {
	panic("not written: r-process")
}

// Start starts the configured command, retrying transient descriptor and busy
// executable errors. The context owns the child's lifetime through Wait.
func (c *Command) Start(ctx context.Context) error { panic("not written: r-process") }

// Wait reaps the command and joins its owned work. Context cancellation kills
// it and its group before joining. The result includes nonzero exit status.
func (c *Command) Wait(ctx context.Context) Exit { panic("not written: r-process") }

// Kill terminates the command and its owned group; an already gone group succeeds.
func (c *Command) Kill() error { panic("not written: r-process") }

// Signal sends a platform-supported signal to the command and its owned group.
func (c *Command) Signal(signal runnerwire.Signal) error { panic("not written: r-process") }

// PID returns the process ID after a successful Start.
func (c *Command) PID() uint32 { panic("not written: r-process") }

// Start retries a single spawn attempt on descriptor exhaustion and briefly
// on a busy executable. Both async and blocking Rust callers use this function.
func Start[T any](ctx context.Context, attempt func() (T, error)) (T, error) {
	panic("not written: r-process")
}
