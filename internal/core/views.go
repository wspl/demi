package core

// ToolView is a tool call's view.
// +demi:union tag=kind
//
//sumtype:decl
type ToolView interface{ isToolView() }

// ShellToolView is a command's status and the end of its output, as the shell tools saw it.
// Its characters are Unicode scalar values, counted from the end.
type ShellToolView struct {
	Status    ShellViewStatus `json:"status"`
	ShellID   ShellID         `json:"shellId"`
	CommandID CommandID       `json:"commandId"`
	// Present once the command exited.
	ExitCode *int32 `json:"exitCode,omitempty"`
	// +demi:range max=9007199254740991
	RunningMs uint64 `json:"runningMs"`
	// +demi:range max=9007199254740991
	IdleMs uint64 `json:"idleMs"`
	// The last 32,768 characters of the merged stdout and stderr.
	Chunks []OutputChunk `json:"chunks"`
	// True when that window or the output itself was cut.
	ViewTruncated bool `json:"viewTruncated"`
	// The files the command changed, once it exited and changed some.
	Files *[]EditedFile `json:"files,omitempty"`
	// Present with `files`: whether the list was cut.
	FilesTruncated *bool `json:"filesTruncated,omitempty"`
}

// RepeatedShellExec is a shell_exec the repeat guard suppressed: the script and how many times in a row it was asked for.
// +demi:variant ToolView repeated_shell_exec
type RepeatedShellExec struct {
	Script string `json:"script"`
	Count  uint32 `json:"count"`
}

// YieldWakeup is the wakeup a yield scheduled.
// +demi:variant ToolView yield_wakeup
type YieldWakeup struct {
	WakeupID   WakeupID `json:"wakeupId"`
	DurationMs uint32   `json:"durationMs"`
}

// ShellViewStatus describes where a command is: running, exited, or stopped.
// +demi:enum running exited aborted
type ShellViewStatus string

// OutputChunk is a run of a command's output from one stream.
type OutputChunk struct {
	Stream StreamKind `json:"stream"`
	Text   string     `json:"text"`
}

// StreamKind is a command's output stream.
// +demi:enum stdout stderr
type StreamKind string

// EditedFile is a file a command changed (`edit-tracking.md` § Edit copies): its line
// counts and, per edit segment, the blobs of its two sides when they were
// stored.
type EditedFile struct {
	// Absolute, as the Host names it.
	// +demi:length min=1
	Path    string   `json:"path"`
	Kind    EditKind `json:"kind"`
	Added   uint32   `json:"added"`
	Removed uint32   `json:"removed"`
	// +demi:length min=1
	Edits []EditSegment `json:"edits"`
}

// EditKind records whether a command created a file or changed one that existed.
// +demi:enum added modified
type EditKind string

// EditSegment is one edit segment of a file.
type EditSegment struct {
	// The file's two sides; absent when they were not stored.
	Copies *EditCopies `json:"copies,omitempty"`
}

// EditCopies is the two sides of an edit segment, blobs of the conversation owner's
// namespace.
type EditCopies struct {
	// The file before the segment; the empty blob when the segment created
	// it.
	Original BlobRef `json:"original"`
	// The file after the segment.
	Modified BlobRef `json:"modified"`
}

// FailureSource describes where a provider failure came from.
// +demi:enum http stream transport unknown
type FailureSource string

// ProviderErrorDiagnostics is the diagnostics of a provider failure, saved with its `error` block. The
// vendor's answer is in `upstream` exactly as it arrived: a stream's frame
// text, or the JSON `{ status, headers, body }` of an HTTP failure; only the
// provider that produced it reads it.
type ProviderErrorDiagnostics struct {
	Source             FailureSource `json:"source"`
	ClientRequestID    *string       `json:"clientRequestId,omitempty"`
	ProviderRequestID  *string       `json:"providerRequestId,omitempty"`
	ProviderResponseID *string       `json:"providerResponseId,omitempty"`
	ProviderCode       *string       `json:"providerCode,omitempty"`
	HTTPStatus         *uint16       `json:"httpStatus,omitempty"`
	Upstream           *string       `json:"upstream,omitempty"`
}

// AgentMessage is a message between agents. The supervisor supplies the sender from the
// invoking node, so a model cannot impersonate another sender.
// +demi:check validateAgentMessage
type AgentMessage struct {
	// The id of the `agent_message` block the message becomes. A
	// completion's id is its [`CompletionId`].
	ID          BlockID `json:"id"`
	Sender      Sender  `json:"sender"`
	RecipientID NodeID  `json:"recipientId"`
	// When the message was sent; for a completion, when the child closed.
	Timestamp Timestamp `json:"timestamp"`
	// The body: a completion's result for `completed`, otherwise its failure
	// text, which can be empty. An explicit message is never empty.
	Content string            `json:"content"`
	Event   AgentMessageEvent `json:"event"`
}

// Sender represents who sent an agent message.
type Sender struct {
	ID NodeID `json:"id"`
	// The number the model knows the sender by: 0 for the root
	// (`runtime.md` § Identifiers the model sees).
	// +demi:range max=9007199254740991
	Number uint64 `json:"number"`
	// The sender's description; `root session` for the root.
	Description string `json:"description"`
	// The sender's round: 1 for its first run, one more at each resume.
	// +demi:range min=1 max=9007199254740991
	Round uint64 `json:"round"`
}

// AgentMessageEvent represents what an agent message is.
// +demi:union tag=type
//
//sumtype:decl
type AgentMessageEvent interface{ isAgentMessageEvent() }

// MessageEvent is an explicit communication between live agents.
// +demi:variant AgentMessageEvent message
type MessageEvent struct{}

// CompletionEvent is a supervisor's receipt of a child's end.
// +demi:variant AgentMessageEvent completion
type CompletionEvent struct {
	Outcome CompletionOutcome `json:"outcome"`
}

// CompletionOutcome represents how a child ended.
// +demi:enum completed failed aborted
type CompletionOutcome string

// ShellView wraps the shell payload as a tagged tool view.
// +demi:variant ToolView shell
type ShellView struct{ ShellToolView }
