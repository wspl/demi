//go:build linux

package system

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"os/exec"
	"syscall"
	"time"
)

// Tool identifies a program the manager runs.
type Tool uint8

const (
	// Runsc is the configured sandbox runtime.
	Runsc Tool = iota
	// Mke2fs creates ext4 images.
	Mke2fs
	// E2fsck checks and repairs ext4 images.
	E2fsck
	// Resize2fs grows ext4 images.
	Resize2fs
	// Bsdtar extracts base archives.
	Bsdtar
	// Nft applies firewall transactions.
	Nft
)

// Name returns the program name.
func (t Tool) Name() string { panic("not written: m-system") }

// String returns the program name.
func (t Tool) String() string { panic("not written: m-system") }

// Tools holds resolved programs. A zero value supplies empty paths for tests
// that only build command lines. Use Resolve for production execution.
type Tools struct{}

// NewTools copies already resolved program paths. It supports fixtures that
// substitute infrastructure programs; production callers use Resolve.
func NewTools(paths map[Tool]string) *Tools { panic("not written: m-system") }

// MissingTools names programs that are not installed.
type MissingTools struct {
	// Names lists missing programs in lookup order.
	Names []string
}

// Error describes the missing programs.
func (e *MissingTools) Error() string { panic("not written: m-system") }

// SpawnError reports a program that could not start or whose wait failed.
type SpawnError struct {
	// Tool identifies the program.
	Tool Tool
	// Source is the start or wait failure.
	Source error
}

// Error describes the failure to run the tool.
func (e *SpawnError) Error() string { panic("not written: m-system") }

// Unwrap preserves the underlying IO error.
func (e *SpawnError) Unwrap() error { panic("not written: m-system") }

// DeadlineError reports a tool that did not finish within its deadline.
type DeadlineError struct {
	// Tool identifies the program.
	Tool Tool
	// Deadline is the permitted runtime.
	Deadline time.Duration
}

// Error describes the tool's exceeded deadline.
func (e *DeadlineError) Error() string { panic("not written: m-system") }

// FailedError reports a program whose exit status was not accepted.
type FailedError struct {
	// Tool identifies the program.
	Tool Tool
	// Output carries the rejected status and decoded output.
	Output Output
}

// Error describes the exit status and the kept tail of output.
func (e *FailedError) Error() string { panic("not written: m-system") }

// Output is what a program wrote, decoded lossily: tools write text.
type Output struct {
	// Status preserves both exit codes and termination by signal.
	Status syscall.WaitStatus
	// Stdout is standard output decoded with replacement for invalid UTF-8.
	Stdout string
	// Stderr is standard error decoded with replacement for invalid UTF-8.
	Stderr string
}

// Message returns stderr, or stdout when stderr is blank, trimmed and bounded
// to its last 8 KiB at a UTF-8 character boundary.
func (o Output) Message() string { panic("not written: m-system") }

// Tail returns the last bytes of text, cut at a UTF-8 character boundary.
func Tail(text string, bytes int) string { panic("not written: m-system") }

// Resolve finds runsc at its configured path and the other programs on PATH.
func Resolve(ctx context.Context, runsc string) (*Tools, error) { panic("not written: m-system") }

// Path returns the resolved path of tool.
func (t *Tools) Path(tool Tool) string { panic("not written: m-system") }

// Command prepares tool with LC_ALL=C.UTF-8 and no input. The caller owns
// Start and Wait, and must cancel and reap a started command on every path.
// Context cancellation kills the child; merely losing its reference does not.
func (t *Tools) Command(ctx context.Context, tool Tool) *exec.Cmd { panic("not written: m-system") }

// Output runs tool to completion and returns output regardless of exit status.
// A nil deadline means no timeout; a non-nil zero duration expires immediately.
// Cancellation or timeout kills and reaps the child before returning.
func (t *Tools) Output(ctx context.Context, tool Tool, args []string, deadline *time.Duration) (Output, error) {
	panic("not written: m-system")
}

// Run runs tool and requires it to exit 0.
func (t *Tools) Run(ctx context.Context, tool Tool, args []string, deadline *time.Duration) (Output, error) {
	panic("not written: m-system")
}

// Accept requires output of tool to have exited with one of codes.
func Accept(tool Tool, output Output, codes []int) (Output, error) { panic("not written: m-system") }
