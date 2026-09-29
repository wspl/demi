package agentproto

import (
	"github.com/wspl/demi/go/core"
)

// A frame the backend sends on a conversation's socket. Every frame for one
// attachment goes through one outbox in causal order.
//
//demi:union tag=type
//demi:export
type ServerFrame interface{ isServerFrame() }

// The connection is attached; the snapshot frames follow.
//
//demi:variant opened open
type ServerFrameOpened struct {
}

func (ServerFrameOpened) isServerFrame() {}

// Answers `edit_and_send` at durable acceptance.
//
//demi:variant edit_result open
type ServerFrameEditResult struct {
	OperationID core.OperationID `json:"operationId" check:"func=core.Validate"`
	Outcome     EditOutcome      `json:"outcome"`
}

func (ServerFrameEditResult) isServerFrame() {}

// A frame the backend refused, and why.
//
//demi:variant rejected open
type ServerFrameRejected struct {
	Command ClientFrameKind `json:"command"`
	Reason  string          `json:"reason"`
}

func (ServerFrameRejected) isServerFrame() {}

// Every block, with the transcript's version.
//
//demi:variant transcript_reset open
type ServerFrameTranscriptReset struct {
	Blocks   []core.Block                          `json:"blocks" check:"each(func=core.Validate)"`
	Version  TranscriptVersion                     `json:"version"`
	Failures *map[string]core.ProviderFailureFacts `json:"failures,omitzero" check:"keys(chars=1..),each(func=core.Validate)"`
}

func (ServerFrameTranscriptReset) isServerFrame() {}

// The patches of one batch; `revision` is one past the previous frame's.
//
//demi:variant transcript_patch open
type ServerFrameTranscriptPatch struct {
	Patches  []TranscriptPatch                     `json:"patches"`
	Revision uint64                                `json:"revision" check:"range=..9007199254740991"`
	Failures *map[string]core.ProviderFailureFacts `json:"failures,omitzero" check:"keys(chars=1..),each(func=core.Validate)"`
}

func (ServerFrameTranscriptPatch) isServerFrame() {}

//demi:variant phase open
type ServerFramePhase struct {
	Phase core.SessionPhase `json:"phase" check:"func=core.Validate"`
}

func (ServerFramePhase) isServerFrame() {}

// The whole queue.
//
//demi:variant queue open
type ServerFrameQueue struct {
	Queue []core.QueuedMessage `json:"queue" check:"each(func=core.Validate)"`
}

func (ServerFrameQueue) isServerFrame() {}

// The complete list of pending steers.
//
//demi:variant pending_steers open
type ServerFramePendingSteers struct {
	PendingSteers []core.PendingSteer `json:"pendingSteers" check:"each(func=core.Validate)"`
}

func (ServerFramePendingSteers) isServerFrame() {}

// Answers `steer` and `steer_queued_message`.
//
//demi:variant steer_result open
type ServerFrameSteerResult struct {
	SteerID core.BlockID `json:"steerId" check:"func=core.Validate"`
	Outcome SteerOutcome `json:"outcome"`
}

func (ServerFrameSteerResult) isServerFrame() {}

// Answers `abort`, in request order.
//
//demi:variant abort_result open
type ServerFrameAbortResult struct {
	Result AbortResult `json:"result"`
}

func (ServerFrameAbortResult) isServerFrame() {}

// A command's live view (`runtime.md` § Live output): the same for
// every page, whatever any page or the model read.
//
//demi:variant shell_output open
type ServerFrameShellOutput struct {
	// The subagent whose command it is; absent for the root's.
	SubagentID *core.NodeID `json:"subagentId,omitzero" check:"func=core.Validate"`
	Status     ShellStatus  `json:"status"`
}

func (ServerFrameShellOutput) isServerFrame() {}

// Acknowledges `shell_write`.
//
//demi:variant shell_write_result open
type ServerFrameShellWriteResult struct {
	CommandID core.CommandID `json:"commandId" check:"func=core.Validate"`
}

func (ServerFrameShellWriteResult) isServerFrame() {}

// A transient provider failure is being retried after `delayMs`.
//
//demi:variant retry_scheduled open
type ServerFrameRetryScheduled struct {
	Attempt     uint32                         `json:"attempt"`
	DelayMs     uint64                         `json:"delayMs" check:"range=..9007199254740991"`
	Code        *string                        `json:"code" check:"nullable"`
	Diagnostics *core.ProviderErrorDiagnostics `json:"diagnostics,omitzero" check:"func=core.Validate"`
}

func (ServerFrameRetryScheduled) isServerFrame() {}

// A failed turn, a refused frame such as one with the code
// `invalid_frame`, or a failed save.
//
//demi:variant error open
type ServerFrameError struct {
	Message     string                         `json:"message"`
	Code        *string                        `json:"code,omitzero"`
	Diagnostics *core.ProviderErrorDiagnostics `json:"diagnostics,omitzero" check:"func=core.Validate"`
}

func (ServerFrameError) isServerFrame() {}

// A subagent started or closed.
//
//demi:variant subagent open
type ServerFrameSubagent struct {
	Event SubagentEvent `json:"event"`
	Job   SubagentJob   `json:"job"`
}

func (ServerFrameSubagent) isServerFrame() {}

//demi:variant subagent_transcript_reset open
type ServerFrameSubagentTranscriptReset struct {
	SubagentID core.NodeID                           `json:"subagentId" check:"func=core.Validate"`
	Blocks     []core.Block                          `json:"blocks" check:"each(func=core.Validate)"`
	Revision   uint64                                `json:"revision" check:"range=..9007199254740991"`
	Failures   *map[string]core.ProviderFailureFacts `json:"failures,omitzero" check:"keys(chars=1..),each(func=core.Validate)"`
}

func (ServerFrameSubagentTranscriptReset) isServerFrame() {}

//demi:variant subagent_transcript_patch open
type ServerFrameSubagentTranscriptPatch struct {
	SubagentID core.NodeID                           `json:"subagentId" check:"func=core.Validate"`
	Patches    []TranscriptPatch                     `json:"patches"`
	Revision   uint64                                `json:"revision" check:"range=..9007199254740991"`
	Failures   *map[string]core.ProviderFailureFacts `json:"failures,omitzero" check:"keys(chars=1..),each(func=core.Validate)"`
}

func (ServerFrameSubagentTranscriptPatch) isServerFrame() {}

// The connection is detached.
//
//demi:variant closed open
type ServerFrameClosed struct {
}

func (ServerFrameClosed) isServerFrame() {}

// Nothing: the connection sent no other frame for 30 seconds. The
// backend's socket sends it, not the tree, so that a page can tell a
// quiet connection from a dead one (`runtime.md` § Order and delivery).
//
//demi:variant heartbeat open
type ServerFrameHeartbeat struct {
}

func (ServerFrameHeartbeat) isServerFrame() {}

// How an edit ended.
//
//demi:union tag=status
//demi:export
type EditOutcome interface{ isEditOutcome() }

// The replacement is durable; its turn has this id.
//
//demi:variant accepted open
type EditOutcomeAccepted struct {
	TurnID core.TurnID `json:"turnId" check:"func=core.Validate"`
}

func (EditOutcomeAccepted) isEditOutcome() {}

//demi:variant rejected open
type EditOutcomeRejected struct {
	Reason string `json:"reason"`
}

func (EditOutcomeRejected) isEditOutcome() {}

// How a steer ended.
//
//demi:union tag=status
//demi:export
type SteerOutcome interface{ isSteerOutcome() }

// The steer is pending until the next continuation boundary.
//
//demi:variant accepted open
type SteerOutcomeAccepted struct {
}

func (SteerOutcomeAccepted) isSteerOutcome() {}

//demi:variant rejected open
type SteerOutcomeRejected struct {
	Reason string `json:"reason"`
}

func (SteerOutcomeRejected) isSteerOutcome() {}

// What an `abort` stopped, and whether another `abort` would stop more.
//
//demi:wire open
type AbortResult struct {
	// Null when there was nothing to stop.
	Target        *AbortTarget `json:"target" check:"nullable"`
	CanAbortAgain bool         `json:"canAbortAgain"`
}

// The one thing an `abort` stops, in the order it looks for one.
//
//demi:enum
//demi:export
type AbortTarget string

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
//
//demi:enum
//demi:export
type SubagentEvent string

const (
	SubagentEventStarted SubagentEvent = "started"
	SubagentEventClosed  SubagentEvent = "closed"
)

// One child agent as the parent's connection sees it.
//
//demi:wire open
type SubagentJob struct {
	SubagentID core.NodeID `json:"subagentId" check:"func=core.Validate"`
	// The node that spawned it: the browser keys nested views by it.
	ParentSessionID core.NodeID `json:"parentSessionId" check:"func=core.Validate"`
	// The spawn's `--description`, or empty.
	Description string `json:"description"`
	// The profile's name; null for the inherit profile.
	Profile *string  `json:"profile" check:"nullable"`
	Phase   JobPhase `json:"phase"`
	// When the round started.
	StartedAt core.Timestamp `json:"startedAt" check:"func=core.Validate"`
	// When it closed; null while it runs.
	EndedAt *core.Timestamp `json:"endedAt" check:"nullable,func=core.Validate"`
	// Only on a `completed` close: the child's last assistant text, at most
	// 32 KiB.
	Result *string `json:"result,omitzero"`
}

// Where a child is.
//
//demi:enum
//demi:export
type JobPhase string

const (
	JobPhaseRunning   JobPhase = "running"
	JobPhaseCompleted JobPhase = "completed"
	JobPhaseAborted   JobPhase = "aborted"
	JobPhaseError     JobPhase = "error"
)

// Where a command is, with the pages' view of it: running, exited with its
// code, or stopped.
//
//demi:union tag=status
//demi:export
type ShellStatus interface {
	isShellStatus()
}

//demi:variant running open
type ShellStatusRunning struct {
	CommandView
}

func (ShellStatusRunning) isShellStatus() {}

//demi:variant exited open
type ShellStatusExited struct {
	CommandView
	ExitCode int32 `json:"exitCode"`
}

func (ShellStatusExited) isShellStatus() {}

//demi:variant aborted open
type ShellStatusAborted struct {
	CommandView
}

func (ShellStatusAborted) isShellStatus() {}

// A command as the pages see it, whatever its status (`runtime.md`
// § Live output).
//
//demi:wire open
type CommandView struct {
	ShellID   core.ShellID   `json:"shellId" check:"func=core.Validate"`
	CommandID core.CommandID `json:"commandId" check:"func=core.Validate"`
	// The `shell_exec` call that started it, in the subagent's transcript
	// when the frame names one.
	ToolUseID string `json:"toolUseId"`
	// The last 4,096 characters of the pages' view of its output: its
	// output in the order it reached the backend, with a note where the
	// runner left some out.
	Tail string `json:"tail"`
	// How many characters the view has held since the command started. A
	// page adds only the characters beyond those it has shown.
	Chars     uint64 `json:"chars" check:"range=..9007199254740991"`
	RunningMs uint64 `json:"runningMs" check:"range=..9007199254740991"`
}

func (v ShellStatusRunning) Command() CommandView { return v.CommandView }

func (v ShellStatusExited) Command() CommandView { return v.CommandView }

func (v ShellStatusAborted) Command() CommandView { return v.CommandView }
