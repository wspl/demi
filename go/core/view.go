package core

//demi:wire
type ShellToolView struct {
	Status         ShellViewStatus `json:"status"`
	ShellID        ShellID         `json:"shellId" check:"func=Validate"`
	CommandID      CommandID       `json:"commandId" check:"func=Validate"`
	ExitCode       *int32          `json:"exitCode,omitzero"`
	RunningMs      uint64          `json:"runningMs" check:"range=..MaxSafeInteger"`
	IdleMs         uint64          `json:"idleMs" check:"range=..MaxSafeInteger"`
	Chunks         []OutputChunk   `json:"chunks"`
	ViewTruncated  bool            `json:"viewTruncated"`
	Files          *[]EditedFile   `json:"files,omitzero"`
	FilesTruncated *bool           `json:"filesTruncated,omitzero"`
}

//demi:wire
type OutputChunk struct {
	Stream StreamKind `json:"stream"`
	Text   string     `json:"text"`
}

//demi:wire
type EditedFile struct {
	Path    string        `json:"path" check:"chars=1.."`
	Kind    EditKind      `json:"kind"`
	Added   uint32        `json:"added"`
	Removed uint32        `json:"removed"`
	Edits   []EditSegment `json:"edits" check:"items=1.."`
}

//demi:wire
type EditSegment struct {
	Copies *EditCopies `json:"copies,omitzero"`
}

//demi:wire
type EditCopies struct {
	Original BlobRef `json:"original" check:"func=Validate"`
	Modified BlobRef `json:"modified" check:"func=Validate"`
}

//demi:wire open
type StreamView struct {
	Offset    uint64 `json:"offset" check:"range=..MaxSafeInteger"`
	Delta     string `json:"delta"`
	Tail      string `json:"tail"`
	Bytes     uint64 `json:"bytes" check:"range=..MaxSafeInteger"`
	Truncated bool   `json:"truncated"`
}

//demi:wire open
type OutputView struct {
	Offset    uint64        `json:"offset" check:"range=..MaxSafeInteger"`
	Line      uint64        `json:"line" check:"range=..MaxSafeInteger"`
	Text      string        `json:"text"`
	Tail      string        `json:"tail"`
	Chunks    []OutputChunk `json:"chunks"`
	Bytes     uint64        `json:"bytes" check:"range=..MaxSafeInteger"`
	Truncated bool          `json:"truncated"`
}

//demi:wire open
type BinaryStdout struct {
	Truncated  bool   `json:"truncated"`
	TotalBytes uint64 `json:"totalBytes" check:"range=..MaxSafeInteger"`
	LimitBytes uint64 `json:"limitBytes" check:"range=..MaxSafeInteger"`
}

//demi:union tag=kind
type ToolView interface{ isToolView() }

//demi:variant shell
type ToolViewShell struct {
	ShellToolView
}

func (ToolViewShell) isToolView() {}

//demi:variant repeated_shell_exec
type ToolViewRepeatedShellExec struct {
	Script string `json:"script"`
	Count  uint32 `json:"count"`
}

func (ToolViewRepeatedShellExec) isToolView() {}

//demi:variant yield_wakeup
type ToolViewYieldWakeup struct {
	WakeupID   WakeupID `json:"wakeupId" check:"func=Validate"`
	DurationMs uint32   `json:"durationMs"`
}

func (ToolViewYieldWakeup) isToolView() {}

//demi:enum
type ShellViewStatus string

const (
	ShellViewStatusRunning ShellViewStatus = "running"
	ShellViewStatusExited  ShellViewStatus = "exited"
	ShellViewStatusAborted ShellViewStatus = "aborted"
)

//demi:enum
type StreamKind string

const (
	StreamKindStdout StreamKind = "stdout"
	StreamKindStderr StreamKind = "stderr"
)

//demi:enum
type EditKind string

const (
	EditKindAdded    EditKind = "added"
	EditKindModified EditKind = "modified"
)
