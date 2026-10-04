package host

//revive:disable:exported
// Contract doc comments below are product text, which does not start with the declared name.

import (
	"context"
	"fmt"
	"time"

	"github.com/wspl/demi/internal/types"
)

// Observation limits and output budgets match the shell tools' defaults.
const (
	MaxObservation          = 600000 * time.Millisecond
	DefaultObservation      = 10000 * time.Millisecond
	DefaultOutputLimitBytes = 1024 * 1024
	DefaultBinaryLimitBytes = 16 * 1024 * 1024
)

// ShellEnvironment owns one node's shells and command handles on one Host.
type ShellEnvironment interface {
	Exec(context.Context, ExecRequest) (CommandStatus, error)
	Status(types.CommandID) (CommandStatus, error)
	ReadOutput(context.Context, types.CommandID) (WholeOutput, error)
	Write(context.Context, types.CommandID, []byte) error
	Abort(context.Context, types.CommandID) error
	PageViews() []PageView
	ReleaseCommand(context.Context, types.CommandID) bool
	DisposeShell(context.Context, types.ShellID) bool
	DisposeAll(context.Context) error
	OwnsShell(types.ShellID) bool
	OwnsCommand(types.CommandID) bool
}

// Numbers assigns conversation sequence numbers once, in order.
type Numbers interface {
	Next(context.Context, types.Sequence) (uint64, error)
}

// PageFeed receives command changes and reports whether a page watches.
// Watching returns an atomic snapshot and a channel closed on the next change;
// callers obtain a new snapshot after notification. No subscription needs releasing.
type PageFeed interface {
	Changed(*CommandRecord)
	Watching() (bool, <-chan struct{})
}

// ExecRequest describes one shell execution. Its context owns the command even after Exec returns.
type ExecRequest struct {
	Script    string
	Shell     ShellTarget
	Window    ObservationWindow
	Caller    JobCaller
	ToolUseID string
}

// ShellTargetKind selects the default, a named idle shell, or an ephemeral shell.
type ShellTargetKind uint8

const (
	DefaultShell ShellTargetKind = iota
	ExistingShell
	EphemeralShell
)

// ShellTarget selects a shell; a nil ephemeral CWD uses the Host default.
type ShellTarget struct {
	Kind ShellTargetKind
	ID   types.ShellID
	CWD  *string
}

// ObservationWindow bounds how long Exec observes a command; its zero value uses the default.
type ObservationWindow struct{ duration time.Duration }

// NewObservationWindow accepts milliseconds from 1 through 600000.
func NewObservationWindow(milliseconds uint64) (ObservationWindow, bool) {
	if milliseconds == 0 || milliseconds > uint64(MaxObservation/time.Millisecond) {
		return ObservationWindow{}, false
	}
	return ObservationWindow{time.Duration(milliseconds) * time.Millisecond}, true
}

// Duration returns the observation duration.
func (w ObservationWindow) Duration() time.Duration {
	if w.duration == 0 {
		return DefaultObservation
	}
	return w.duration
}

// The command storage a job's `rpc` calls reach: the calling node and the
// history generation it was at when the job started
// (`command-state-history.md` § Mutation API and concurrency).
// +demi:root
type JobCaller struct {
	Node       types.NodeID `json:"node"`
	Generation uint64       `json:"generation"`
}

// CommandStatus holds the model's status and output since its last look.
type CommandStatus struct {
	ShellID           types.ShellID
	CommandID         types.CommandID
	Stdout, Stderr    types.StreamView
	Output            types.OutputView
	Unreceived        uint64
	Newest            []Newest
	Whole             *WholeView
	RunningMs, IdleMs uint64
	State             CommandState
	Files             *EditedFiles
}

// Newest holds a stream's newest bytes beyond its received start.
type Newest struct {
	Stream          types.StreamKind
	Offset, LeftOut uint64
	Text            string
}

// Phase identifies a command's mutually exclusive lifecycle state.
type Phase uint8

const (
	Running Phase = iota
	Exited
	Aborted
)

// CommandState reports the running hint or final exit and binary stdout.
type CommandState struct {
	Phase        Phase
	Hint         *string
	ExitCode     int32
	BinaryStdout *BinaryOutput
}

// WholeView reports the whole output and how much the model already saw.
type WholeView struct {
	Output *WholeOutput
	Seen   Seen
}

// BinaryOutput holds final non-text stdout if complete and within its limit.
type BinaryOutput struct {
	Bytes []byte
	Info  types.BinaryStdout
}

// EditedFiles reports files changed for the user, never the model.
type EditedFiles struct {
	Files     []types.EditedFile
	Truncated bool
}

// ShellErrorKind names why an environment refused a request.
type ShellErrorKind uint8

const (
	UnknownShell ShellErrorKind = iota
	UnknownCommand
	ShellBusy
	NotRunning
	EmptyStdin
)

// ShellError carries the shell tool's model-facing refusal.
type ShellError struct {
	Kind    ShellErrorKind
	Shell   types.ShellID
	Command types.CommandID
}

// Error returns the failure message.
func (e *ShellError) Error() string {
	switch e.Kind {
	case UnknownShell:
		return fmt.Sprintf("Unknown shell session %q", e.Shell)
	case UnknownCommand:
		return fmt.Sprintf("Unknown command %q", e.Command)
	case ShellBusy:
		return fmt.Sprintf("Shell session %q is already running command %q", e.Shell, e.Command)
	case NotRunning:
		return fmt.Sprintf("Command %q is not running", e.Command)
	case EmptyStdin:
		return `shell_write field "stdin" must not be empty; use shell_status to poll`
	}
	return ""
}
