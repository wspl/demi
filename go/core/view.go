package core

// A command's status and the end of its output, as the shell tools saw it.
// Its characters are Unicode scalar values, counted from the end.
//
//demi:wire
type ShellToolView struct {
	Status    ShellViewStatus `json:"status"`
	ShellID   ShellID         `json:"shellId" check:"func=Validate"`
	CommandID CommandID       `json:"commandId" check:"func=Validate"`
	// Present once the command exited.
	ExitCode  *int32 `json:"exitCode,omitzero"`
	RunningMs uint64 `json:"runningMs" check:"range=..MaxSafeInteger"`
	IdleMs    uint64 `json:"idleMs" check:"range=..MaxSafeInteger"`
	// The last 32,768 characters of the merged stdout and stderr.
	Chunks []OutputChunk `json:"chunks"`
	// True when that window or the output itself was cut.
	ViewTruncated bool `json:"viewTruncated"`
	// The files the command changed, once it exited and changed some.
	Files *[]EditedFile `json:"files,omitzero"`
	// Present with `files`: whether the list was cut.
	FilesTruncated *bool `json:"filesTruncated,omitzero"`
}

// A run of a command's output from one stream.
//
//demi:wire
type OutputChunk struct {
	Stream StreamKind `json:"stream"`
	Text   string     `json:"text"`
}

// A file a command changed (`edit-tracking.md` § Edit copies): its line
// counts and, per edit segment, the blobs of its two sides when they were
// stored.
//
//demi:wire
type EditedFile struct {
	// Absolute, as the Host names it.
	Path    string        `json:"path" check:"chars=1.."`
	Kind    EditKind      `json:"kind"`
	Added   uint32        `json:"added"`
	Removed uint32        `json:"removed"`
	Edits   []EditSegment `json:"edits" check:"items=1.."`
}

// One edit segment of a file.
//
//demi:wire
type EditSegment struct {
	// The file's two sides; absent when they were not stored.
	Copies *EditCopies `json:"copies,omitzero"`
}

// The two sides of an edit segment, blobs of the conversation owner's
// namespace.
//
//demi:wire
type EditCopies struct {
	// The file before the segment; the empty blob when the segment created
	// it.
	Original BlobRef `json:"original" check:"func=Validate"`
	// The file after the segment.
	Modified BlobRef `json:"modified" check:"func=Validate"`
}

// One output stream of a command since the last look.
//
//demi:wire open
type StreamView struct {
	// Where the next look starts, in bytes: just after `delta`.
	Offset uint64 `json:"offset" check:"range=..MaxSafeInteger"`
	// The text since the last look.
	Delta string `json:"delta"`
	// The end of the stream.
	Tail string `json:"tail"`
	// The stream's length so far, in bytes.
	Bytes     uint64 `json:"bytes" check:"range=..MaxSafeInteger"`
	Truncated bool   `json:"truncated"`
}

// A command's merged stdout and stderr since the last look.
//
//demi:wire open
type OutputView struct {
	// Where the next model look starts, in bytes: after the whole lines in
	// `text`, or at the start of its unfinished last line while it runs.
	Offset uint64 `json:"offset" check:"range=..MaxSafeInteger"`
	// The line of the merged output that `text` starts in, from 1.
	Line uint64 `json:"line" check:"range=..MaxSafeInteger"`
	// The merged text since the last look, repeating an unfinished line.
	Text      string        `json:"text"`
	Tail      string        `json:"tail"`
	Chunks    []OutputChunk `json:"chunks"`
	Bytes     uint64        `json:"bytes" check:"range=..MaxSafeInteger"`
	Truncated bool          `json:"truncated"`
}

// A command's final stdout that was not text, described by its size: its
// bytes never travel in a frame.
//
//demi:wire open
type BinaryStdout struct {
	// True when the stream exceeded `limitBytes` and was cut.
	Truncated bool `json:"truncated"`
	// The stream's whole length.
	TotalBytes uint64 `json:"totalBytes" check:"range=..MaxSafeInteger"`
	// The ceiling that applied.
	LimitBytes uint64 `json:"limitBytes" check:"range=..MaxSafeInteger"`
}

// A tool call's view.
//
//demi:union tag=kind
//demi:export
type ToolView interface{ isToolView() }

// A shell tool's command.
//
//demi:variant shell
type ToolViewShell struct {
	ShellToolView
}

func (ToolViewShell) isToolView() {}

// A `shell_exec` the repeat guard suppressed: the script and how many
// times in a row it was asked for.
//
//demi:variant repeated_shell_exec
type ToolViewRepeatedShellExec struct {
	Script string `json:"script"`
	Count  uint32 `json:"count"`
}

func (ToolViewRepeatedShellExec) isToolView() {}

// The wakeup a `yield` scheduled.
//
//demi:variant yield_wakeup
type ToolViewYieldWakeup struct {
	WakeupID   WakeupID `json:"wakeupId" check:"func=Validate"`
	DurationMs uint32   `json:"durationMs"`
}

func (ToolViewYieldWakeup) isToolView() {}

// Where a command is: running, exited, or stopped.
//
//demi:enum
//demi:export
type ShellViewStatus string

const (
	ShellViewStatusRunning ShellViewStatus = "running"
	ShellViewStatusExited  ShellViewStatus = "exited"
	ShellViewStatusAborted ShellViewStatus = "aborted"
)

// A command's output stream.
//
//demi:enum
//demi:export
type StreamKind string

const (
	StreamKindStdout StreamKind = "stdout"
	StreamKindStderr StreamKind = "stderr"
)

// Whether a command created a file or changed one that existed.
//
//demi:enum
//demi:export
type EditKind string

const (
	EditKindAdded    EditKind = "added"
	EditKindModified EditKind = "modified"
)
