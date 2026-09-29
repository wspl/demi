package shell

import (
	"context"
	"fmt"
	"time"

	"github.com/wspl/demi/go/core"
)

const (
	MaxObservation          = 600000 * time.Millisecond
	DefaultObservation      = 10000 * time.Millisecond
	DefaultOutputLimitBytes = 1024 * 1024
	DefaultBinaryLimitBytes = 16 * 1024 * 1024
)

// ShellEnvironment's records and handles belong to this environment alone.
// Context cancellation of Exec stops its command even after Exec returns a
// running status. DisposeAll joins all work the environment started.
type ShellEnvironment interface {
	Exec(context.Context, ExecRequest) (CommandStatus, error)
	Status(core.CommandID) (CommandStatus, error)
	ReadOutput(context.Context, core.CommandID) (*WholeOutput, error)
	Write(context.Context, core.CommandID, []byte) error
	Abort(context.Context, core.CommandID) error
	PageViews() []PageView
	ReleaseCommand(context.Context, core.CommandID) bool
	DisposeShell(context.Context, core.ShellID) bool
	DisposeAll(context.Context) error
	OwnsShell(core.ShellID) bool
	OwnsCommand(core.CommandID) bool
}
type Numbers interface {
	Next(context.Context, core.Sequence) (uint64, error)
}

// PageFeed receives immutable views; reads never move the model's cursor.
// Watching returns the current value and a notification closed on its next
// change. The feed owns notifications; callers re-read after each change.
type PageFeed interface {
	Changed(PageView)
	Watching() (bool, <-chan struct{})
}
type ExecRequest struct {
	Script    string
	Shell     ShellTarget
	Window    ObservationWindow
	Caller    JobCaller
	ToolUseID string
}
type ShellTargetKind string

const (
	ShellDefault   ShellTargetKind = ""
	ShellExisting  ShellTargetKind = "existing"
	ShellEphemeral ShellTargetKind = "ephemeral"
)

type ShellTarget struct {
	Kind ShellTargetKind
	ID   core.ShellID
	Cwd  *string
}

// ObservationWindow's zero value is the default ten-second observation.
type ObservationWindow struct{ duration time.Duration }

func NewObservationWindow(milliseconds uint64) (ObservationWindow, bool) {
	if milliseconds == 0 || milliseconds > 600000 {
		return ObservationWindow{}, false
	}
	return ObservationWindow{time.Duration(milliseconds) * time.Millisecond}, true
}
func (w ObservationWindow) Duration() time.Duration {
	if w.duration == 0 {
		return DefaultObservation
	}
	return w.duration
}

//demi:wire
type JobCaller struct {
	Node       core.NodeID `json:"node" check:"func=core.Validate"`
	Generation uint64      `json:"generation"`
}
type CommandStatus struct {
	ShellID    core.ShellID
	CommandID  core.CommandID
	Stdout     core.StreamView
	Stderr     core.StreamView
	Output     core.OutputView
	Unreceived uint64
	Whole      *WholeView
	RunningMs  uint64
	IdleMs     uint64
	State      CommandState
	Files      *EditedFiles
}
type CommandPhase string

const (
	CommandRunning CommandPhase = "running"
	CommandExited  CommandPhase = "exited"
	CommandAborted CommandPhase = "aborted"
)

type CommandState struct {
	Phase        CommandPhase
	Hint         *string
	ExitCode     int32
	BinaryStdout *BinaryOutput
}
type WholeView struct {
	Output *WholeOutput
	Seen   Seen
}
type BinaryOutput struct {
	Bytes []byte
	Info  core.BinaryStdout
}
type EditedFiles struct {
	Files     []core.EditedFile
	Truncated bool
}

type ShellErrorKind string

const (
	UnknownShell   ShellErrorKind = "unknown_shell"
	UnknownCommand ShellErrorKind = "unknown_command"
	ShellBusy      ShellErrorKind = "shell_busy"
	NotRunning     ShellErrorKind = "not_running"
	EmptyStdin     ShellErrorKind = "empty_stdin"
	Starting       ShellErrorKind = "starting"
)

type ShellError struct {
	Kind    ShellErrorKind
	Shell   core.ShellID
	Command core.CommandID
}

func (e *ShellError) Error() string {
	switch e.Kind {
	case UnknownShell:
		return fmt.Sprintf("Unknown shell session \"%s\"", e.Shell.String())
	case UnknownCommand:
		return fmt.Sprintf("Unknown command \"%s\"", e.Command.String())
	case ShellBusy:
		return fmt.Sprintf("Shell session \"%s\" is already running command \"%s\"", e.Shell.String(), e.Command.String())
	case NotRunning:
		return fmt.Sprintf("Command \"%s\" is not running", e.Command.String())
	case EmptyStdin:
		return `shell_write field "stdin" must not be empty; use shell_status to poll`
	case Starting:
		return fmt.Sprintf("Command \"%s\" is still acquiring its Host", e.Command.String())
	}
	return string(e.Kind)
}
