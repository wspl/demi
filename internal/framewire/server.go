package framewire

import "github.com/wspl/demi/internal/core"

// A frame the backend sends on a conversation's socket. Every frame for one
// attachment goes through one outbox in causal order.
// +demi:root direction=receive output=protocol
// +demi:union tag=type
//
//sumtype:decl
type ServerFrame interface {
	isServerFrame()
}

// The connection is attached; the snapshot frames follow.
// +demi:variant ServerFrame opened
// +demi:tolerant
type OpenedFrame struct {
}

// Answers `edit_and_send` at durable acceptance.
// +demi:variant ServerFrame edit_result
// +demi:tolerant
type EditResultFrame struct {
	OperationID core.OperationID `json:"operationId"`
	Outcome     EditOutcome      `json:"outcome"`
}

// A frame the backend refused, and why.
// +demi:variant ServerFrame rejected
// +demi:tolerant
type RejectedFrame struct {
	Command ClientFrameKind `json:"command"`
	Reason  string          `json:"reason"`
}

// Every block, with the transcript's version.
// +demi:variant ServerFrame transcript_reset
// +demi:tolerant
type TranscriptResetFrame struct {
	Blocks   []core.Block      `json:"blocks"`
	Version  TranscriptVersion `json:"version"`
	Failures *Failures         `json:"failures,omitempty"`
}

// The patches of one batch; `revision` is one past the previous frame's.
// +demi:variant ServerFrame transcript_patch
// +demi:tolerant
type TranscriptPatchFrame struct {
	Patches []TranscriptPatch `json:"patches"`
	// +demi:range max=9007199254740991
	Revision uint64    `json:"revision"`
	Failures *Failures `json:"failures,omitempty"`
}

// +demi:variant ServerFrame phase
// +demi:tolerant
type PhaseFrame struct {
	Phase core.SessionPhase `json:"phase"`
}

// The whole queue.
// +demi:variant ServerFrame queue
// +demi:tolerant
type QueueFrame struct {
	Queue []core.QueuedMessage `json:"queue"`
}

// The complete list of pending steers.
// +demi:variant ServerFrame pending_steers
// +demi:tolerant
type PendingSteersFrame struct {
	PendingSteers []core.PendingSteer `json:"pendingSteers"`
}

// Answers `steer` and `steer_queued_message`.
// +demi:variant ServerFrame steer_result
// +demi:tolerant
type SteerResultFrame struct {
	SteerID core.BlockID `json:"steerId"`
	Outcome SteerOutcome `json:"outcome"`
}

// Answers `abort`, in request order.
// +demi:variant ServerFrame abort_result
// +demi:tolerant
type AbortResultFrame struct {
	Result AbortResult `json:"result"`
}

// A command's live view (`runtime.md` § Live output): the same for
// every page, whatever any page or the model read.
// +demi:variant ServerFrame shell_output
// +demi:tolerant
type ShellOutputFrame struct {
	// The subagent whose command it is; absent for the root's.
	SubagentID *core.NodeID `json:"subagentId,omitempty"`
	Status     ShellStatus  `json:"status"`
}

// Acknowledges `shell_write`.
// +demi:variant ServerFrame shell_write_result
// +demi:tolerant
type ShellWriteResultFrame struct {
	CommandID core.CommandID `json:"commandId"`
}

// A transient provider failure is being retried after `delayMs`.
// +demi:variant ServerFrame retry_scheduled
// +demi:tolerant
type RetryScheduledFrame struct {
	Attempt uint32 `json:"attempt"`
	// +demi:range max=9007199254740991
	DelayMs uint64 `json:"delayMs"`
	// +demi:nullable
	Code        *string                        `json:"code"`
	Diagnostics *core.ProviderErrorDiagnostics `json:"diagnostics,omitempty"`
}

// A failed turn, a refused frame such as one with the code
// `invalid_frame`, or a failed save.
// +demi:variant ServerFrame error
// +demi:tolerant
type ErrorFrame struct {
	Message     string                         `json:"message"`
	Code        *string                        `json:"code,omitempty"`
	Diagnostics *core.ProviderErrorDiagnostics `json:"diagnostics,omitempty"`
}

// A subagent started or closed.
// +demi:variant ServerFrame subagent
// +demi:tolerant
type SubagentFrame struct {
	Event SubagentEvent `json:"event"`
	Job   SubagentJob   `json:"job"`
}

// +demi:variant ServerFrame subagent_transcript_reset
// +demi:tolerant
type SubagentTranscriptResetFrame struct {
	SubagentID core.NodeID  `json:"subagentId"`
	Blocks     []core.Block `json:"blocks"`
	// +demi:range max=9007199254740991
	Revision uint64    `json:"revision"`
	Failures *Failures `json:"failures,omitempty"`
}

// +demi:variant ServerFrame subagent_transcript_patch
// +demi:tolerant
type SubagentTranscriptPatchFrame struct {
	SubagentID core.NodeID       `json:"subagentId"`
	Patches    []TranscriptPatch `json:"patches"`
	// +demi:range max=9007199254740991
	Revision uint64    `json:"revision"`
	Failures *Failures `json:"failures,omitempty"`
}

// The connection is detached.
// +demi:variant ServerFrame closed
// +demi:tolerant
type ClosedFrame struct {
}

// Nothing: the connection sent no other frame for 30 seconds. The
// backend's socket sends it, not the tree, so that a page can tell a
// quiet connection from a dead one (`runtime.md` § Order and delivery).
// +demi:variant ServerFrame heartbeat
// +demi:tolerant
type HeartbeatFrame struct {
}

// How an edit ended.
// +demi:union tag=status
//
//sumtype:decl
type EditOutcome interface {
	isEditOutcome()
}

// The replacement is durable; its turn has this id.
// +demi:variant EditOutcome accepted
// +demi:tolerant
type AcceptedEdit struct {
	TurnID core.TurnID `json:"turnId"`
}

// +demi:variant EditOutcome rejected
// +demi:tolerant
type RejectedEdit struct {
	Reason string `json:"reason"`
}

// How a steer ended.
// +demi:union tag=status
//
//sumtype:decl
type SteerOutcome interface {
	isSteerOutcome()
}

// The steer is pending until the next continuation boundary.
// +demi:variant SteerOutcome accepted
// +demi:tolerant
type AcceptedSteer struct {
}

// +demi:variant SteerOutcome rejected
// +demi:tolerant
type RejectedSteer struct {
	Reason string `json:"reason"`
}

// What an `abort` stopped, and whether another `abort` would stop more.
// +demi:tolerant
type AbortResult struct {
	// Null when there was nothing to stop.
	// +demi:nullable
	Target        *AbortTarget `json:"target"`
	CanAbortAgain bool         `json:"canAbortAgain"`
}

// The one thing an `abort` stops, in the order it looks for one.
// +demi:enum active_provider_stream active_tool active_compaction active_turn queued_action queued_message pending_yield_wakeup
type AbortTarget string

// Abort targets follow the order in which an abort looks for work.
const (
	AbortTargetActiveProviderStream AbortTarget = "active_provider_stream"
	AbortTargetActiveTool           AbortTarget = "active_tool"
	AbortTargetActiveCompaction     AbortTarget = "active_compaction"
	AbortTargetActiveTurn           AbortTarget = "active_turn"
	AbortTargetQueuedAction         AbortTarget = "queued_action"
	AbortTargetQueuedMessage        AbortTarget = "queued_message"
	AbortTargetPendingYieldWakeup   AbortTarget = "pending_yield_wakeup"
)

// Whether a subagent started or closed.
// +demi:enum started closed
type SubagentEvent string

// Subagent events announce child lifecycle changes.
const (
	SubagentEventStarted SubagentEvent = "started"
	SubagentEventClosed  SubagentEvent = "closed"
)

// One child agent as the parent's connection sees it.
// +demi:tolerant
type SubagentJob struct {
	SubagentID core.NodeID `json:"subagentId"`
	// The node that spawned it: the web app keys nested views by it.
	ParentSessionID core.NodeID `json:"parentSessionId"`
	// The spawn's `--description`, or empty.
	Description string `json:"description"`
	// The profile's name; null for the inherit profile.
	// +demi:nullable
	Profile *string  `json:"profile"`
	Phase   JobPhase `json:"phase"`
	// When the round started.
	StartedAt core.Timestamp `json:"startedAt"`
	// When it closed; null while it runs.
	// +demi:nullable
	EndedAt *core.Timestamp `json:"endedAt"`
	// Only on a `completed` close: the child's last assistant text, at most
	// 32 KiB.
	Result *string `json:"result,omitempty"`
}

// Where a child is.
// +demi:enum running completed aborted error
type JobPhase string

// Job phases describe a child agent.
const (
	JobPhaseRunning   JobPhase = "running"
	JobPhaseCompleted JobPhase = "completed"
	JobPhaseAborted   JobPhase = "aborted"
	JobPhaseError     JobPhase = "error"
)

// Where a command is, with the pages' view of it: running, exited with its
// code, or stopped.
// +demi:union tag=status
//
//sumtype:decl
type ShellStatus interface {
	isShellStatus()
	Command() *CommandView
}

// +demi:variant ShellStatus running
// +demi:tolerant
type RunningStatus struct {
	CommandView
}

// Command returns the command and its view, whatever its status.
func (v *RunningStatus) Command() *CommandView { return &v.CommandView }

// +demi:variant ShellStatus exited
// +demi:tolerant
type ExitedStatus struct {
	CommandView
	ExitCode int32 `json:"exitCode"`
}

// Command returns the command and its view, whatever its status.
func (v *ExitedStatus) Command() *CommandView { return &v.CommandView }

// +demi:variant ShellStatus aborted
// +demi:tolerant
type AbortedStatus struct {
	CommandView
}

// Command returns the command and its view, whatever its status.
func (v *AbortedStatus) Command() *CommandView { return &v.CommandView }

// A command as the pages see it, whatever its status (`runtime.md`
// § Live output).
// +demi:tolerant
type CommandView struct {
	ShellID   core.ShellID   `json:"shellId"`
	CommandID core.CommandID `json:"commandId"`
	// The `shell_exec` call that started it, in the subagent's transcript
	// when the frame names one.
	ToolUseID string `json:"toolUseId"`
	// The last 4,096 characters of the pages' view of its output: its
	// output in the order it reached the backend, with a note where the
	// runner left some out.
	Tail string `json:"tail"`
	// How many characters the view has held since the command started. A
	// page adds only the characters beyond those it has shown.
	// +demi:range max=9007199254740991
	Chars uint64 `json:"chars"`
	// +demi:range max=9007199254740991
	RunningMs uint64 `json:"runningMs"`
}
