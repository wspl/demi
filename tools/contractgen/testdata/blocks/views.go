package contracts

// A tool call's view.
// +demi:union tag=kind
//
//sumtype:decl
type ToolView interface{ isToolView() }

// A command's status and the end of its output, as the shell tools saw it.
// Its characters are Unicode scalar values, counted from the end.
// +demi:variant ToolView shell
type ShellToolView struct {
	Status    ShellViewStatus `json:"status"`
	ShellID   ShellId         `json:"shellId"`
	CommandID CommandId       `json:"commandId"`
	// Present once the command exited.
	ExitCode *int32 `json:"exitCode,omitzero"`
	// +demi:range max=9007199254740991
	RunningMs uint64 `json:"runningMs"`
	// +demi:range max=9007199254740991
	IdleMs uint64 `json:"idleMs"`
	// The last 32,768 characters of the merged stdout and stderr.
	Chunks []OutputChunk `json:"chunks"`
	// True when that window or the output itself was cut.
	ViewTruncated bool `json:"viewTruncated"`
	// The files the command changed, once it exited and changed some.
	Files *[]EditedFile `json:"files,omitzero"`
	// Present with `files`: whether the list was cut.
	FilesTruncated *bool `json:"filesTruncated,omitzero"`
}

// A shell_exec the repeat guard suppressed: the script and how many times in a row it was asked for.
// +demi:variant ToolView repeated_shell_exec
type RepeatedShellExec struct {
	Script string `json:"script"`
	Count  uint32 `json:"count"`
}

// The wakeup a yield scheduled.
// +demi:variant ToolView yield_wakeup
type YieldWakeup struct {
	WakeupID   WakeupId `json:"wakeupId"`
	DurationMs uint32   `json:"durationMs"`
}

// Where a command is: running, exited, or stopped.
// +demi:enum running exited aborted
type ShellViewStatus string

// A run of a command's output from one stream.
type OutputChunk struct {
	Stream StreamKind `json:"stream"`
	Text   string     `json:"text"`
}

// A command's output stream.
// +demi:enum stdout stderr
type StreamKind string

// A file a command changed (`edit-tracking.md` § Edit copies): its line
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

// Whether a command created a file or changed one that existed.
// +demi:enum added modified
type EditKind string

// One edit segment of a file.
type EditSegment struct {
	// The file's two sides; absent when they were not stored.
	Copies *EditCopies `json:"copies,omitzero"`
}

// The two sides of an edit segment, blobs of the conversation owner's
// namespace.
type EditCopies struct {
	// The file before the segment; the empty blob when the segment created
	// it.
	Original BlobRef `json:"original"`
	// The file after the segment.
	Modified BlobRef `json:"modified"`
}

// Where a provider failure came from.
// +demi:enum http stream transport unknown
type FailureSource string

// The diagnostics of a provider failure, saved with its `error` block. The
// vendor's answer is in `upstream` exactly as it arrived: a stream's frame
// text, or the JSON `{ status, headers, body }` of an HTTP failure; only the
// provider that produced it reads it.
type ProviderErrorDiagnostics struct {
	Source             FailureSource `json:"source"`
	ClientRequestID    *string       `json:"clientRequestId,omitzero"`
	ProviderRequestID  *string       `json:"providerRequestId,omitzero"`
	ProviderResponseID *string       `json:"providerResponseId,omitzero"`
	ProviderCode       *string       `json:"providerCode,omitzero"`
	HTTPStatus         *uint16       `json:"httpStatus,omitzero"`
	Upstream           *string       `json:"upstream,omitzero"`
}

// A message between agents. The supervisor supplies the sender from the
// invoking node, so a model cannot impersonate another sender.
// +demi:check validateAgentMessage
type AgentMessage struct {
	// The id of the `agent_message` block the message becomes. A
	// completion's id is `subagent:<child id>:<round>`.
	ID          BlockId `json:"id"`
	Sender      Sender  `json:"sender"`
	RecipientID NodeId  `json:"recipientId"`
	// When the message was sent; for a completion, when the child closed.
	Timestamp Timestamp `json:"timestamp"`
	// The body: a completion's result for `completed`, otherwise its failure
	// text, which can be empty. An explicit message is never empty.
	Content string            `json:"content"`
	Event   AgentMessageEvent `json:"event"`
}

// Who sent an agent message.
type Sender struct {
	ID NodeId `json:"id"`
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

// What an agent message is.
// +demi:union tag=type
//
//sumtype:decl
type AgentMessageEvent interface{ isAgentMessageEvent() }

// An explicit communication between live agents.
// +demi:variant AgentMessageEvent message
type MessageEvent struct{}

// A supervisor's receipt of a child's end.
// +demi:variant AgentMessageEvent completion
type CompletionEvent struct {
	Outcome CompletionOutcome `json:"outcome"`
}

// How a child ended.
// +demi:enum completed failed aborted
type CompletionOutcome string
