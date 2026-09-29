package agentproto

import (
	"github.com/wspl/demi/go/core"
)

// A frame the browser sends. The connection belongs to one conversation, so
// no frame names a session or a working directory.
//
//demi:union tag=type
//demi:export
type ClientFrame interface {
	isClientFrame()
}

// Attach this connection to the conversation's tree, restoring it when
// it is not live with the model selection the conversation's record
// holds; no frame names a model (`runtime.md` § Model switch).
//
//demi:variant open
type ClientFrameOpen struct {
}

func (ClientFrameOpen) isClientFrame() {}

// Submit a message; its id becomes its turn's id and its queue entry's.
//
//demi:variant send
type ClientFrameSend struct {
	MessageID core.TurnID     `json:"messageId" check:"func=core.Validate"`
	Content   []ClientContent `json:"content"`
}

func (ClientFrameSend) isClientFrame() {}

// Replace a user message and everything after it.
//
//demi:variant edit_and_send
type ClientFrameEditAndSend struct {
	Request EditRequest `json:"request"`
}

func (ClientFrameEditAndSend) isClientFrame() {}

// Add input to the running turn; its id is its `steer` block's.
//
//demi:variant steer
type ClientFrameSteer struct {
	SteerID core.BlockID    `json:"steerId" check:"func=core.Validate"`
	Content []ClientContent `json:"content"`
}

func (ClientFrameSteer) isClientFrame() {}

//demi:variant cancel_pending_steer
type ClientFrameCancelPendingSteer struct {
	SteerID core.BlockID `json:"steerId" check:"func=core.Validate"`
}

func (ClientFrameCancelPendingSteer) isClientFrame() {}

//demi:variant dequeue_message
type ClientFrameDequeueMessage struct {
	MessageID core.TurnID `json:"messageId" check:"func=core.Validate"`
}

func (ClientFrameDequeueMessage) isClientFrame() {}

// Move a queued message to the front, so that it runs next.
//
//demi:variant send_queued_message
type ClientFrameSendQueuedMessage struct {
	MessageID core.TurnID `json:"messageId" check:"func=core.Validate"`
}

func (ClientFrameSendQueuedMessage) isClientFrame() {}

// Turn a queued message into a steer of the running turn.
//
//demi:variant steer_queued_message
type ClientFrameSteerQueuedMessage struct {
	MessageID core.TurnID  `json:"messageId" check:"func=core.Validate"`
	SteerID   core.BlockID `json:"steerId" check:"func=core.Validate"`
}

func (ClientFrameSteerQueuedMessage) isClientFrame() {}

//demi:variant clear_message_queue
type ClientFrameClearMessageQueue struct {
}

func (ClientFrameClearMessageQueue) isClientFrame() {}

// Stop one thing (`runtime.md` § Stop).
//
//demi:variant abort
type ClientFrameAbort struct {
}

func (ClientFrameAbort) isClientFrame() {}

//demi:variant abort_subagents
type ClientFrameAbortSubagents struct {
}

func (ClientFrameAbortSubagents) isClientFrame() {}

// Stop one live subagent with its subtree.
//
//demi:variant abort_subagent
type ClientFrameAbortSubagent struct {
	SubagentID core.NodeID `json:"subagentId" check:"func=core.Validate"`
}

func (ClientFrameAbortSubagent) isClientFrame() {}

//demi:variant retry
type ClientFrameRetry struct {
}

func (ClientFrameRetry) isClientFrame() {}

//demi:variant resume
type ClientFrameResume struct {
}

func (ClientFrameResume) isClientFrame() {}

//demi:variant compact
type ClientFrameCompact struct {
}

func (ClientFrameCompact) isClientFrame() {}

// Write stdin to a running command.
//
//demi:variant shell_write
type ClientFrameShellWrite struct {
	CommandID core.CommandID `json:"commandId" check:"func=core.Validate"`
	Stdin     string         `json:"stdin"`
}

func (ClientFrameShellWrite) isClientFrame() {}

// Stop a running command.
//
//demi:variant shell_abort
type ClientFrameShellAbort struct {
	CommandID core.CommandID `json:"commandId" check:"func=core.Validate"`
}

func (ClientFrameShellAbort) isClientFrame() {}

// Ask for a fresh transcript, after a gap in the patch revisions.
//
//demi:variant sync_transcript
type ClientFrameSyncTranscript struct {
}

func (ClientFrameSyncTranscript) isClientFrame() {}

// Dispose the tree.
//
//demi:variant close
type ClientFrameClose struct {
}

func (ClientFrameClose) isClientFrame() {}

// Which frame a frame is: its `type`, which a `rejected` frame names.
//
//demi:enum
//demi:export
type ClientFrameKind string

const (
	ClientFrameKindOpen               ClientFrameKind = "open"
	ClientFrameKindSend               ClientFrameKind = "send"
	ClientFrameKindEditAndSend        ClientFrameKind = "edit_and_send"
	ClientFrameKindSteer              ClientFrameKind = "steer"
	ClientFrameKindCancelPendingSteer ClientFrameKind = "cancel_pending_steer"
	ClientFrameKindDequeueMessage     ClientFrameKind = "dequeue_message"
	ClientFrameKindSendQueuedMessage  ClientFrameKind = "send_queued_message"
	ClientFrameKindSteerQueuedMessage ClientFrameKind = "steer_queued_message"
	ClientFrameKindClearMessageQueue  ClientFrameKind = "clear_message_queue"
	ClientFrameKindAbort              ClientFrameKind = "abort"
	ClientFrameKindAbortSubagents     ClientFrameKind = "abort_subagents"
	ClientFrameKindAbortSubagent      ClientFrameKind = "abort_subagent"
	ClientFrameKindRetry              ClientFrameKind = "retry"
	ClientFrameKindResume             ClientFrameKind = "resume"
	ClientFrameKindCompact            ClientFrameKind = "compact"
	ClientFrameKindShellWrite         ClientFrameKind = "shell_write"
	ClientFrameKindShellAbort         ClientFrameKind = "shell_abort"
	ClientFrameKindSyncTranscript     ClientFrameKind = "sync_transcript"
	ClientFrameKindClose              ClientFrameKind = "close"
)

// A replacement of a user message and everything after it (`message-editing.md`).
//
//demi:wire
type EditRequest struct {
	// Chosen by the browser, so a repeated request is recognized.
	OperationID core.OperationID `json:"operationId" check:"func=core.Validate"`
	// The `user` block the edit replaces.
	TargetBlockID core.BlockID `json:"targetBlockId" check:"func=core.Validate"`
	// The transcript version the editor showed; a stale one is refused.
	Version TranscriptVersion `json:"version"`
	// The complete replacement content.
	Content []ClientContent `json:"content" check:"items=1.."`
}

// One part of the content the browser sends. Files are referred to, never
// carried: a new file is an upload or a file on a paired device, and an edit
// keeps what the edited message holds by reference.
//
//demi:union tag=type
//demi:export
type ClientContent interface{ isClientContent() }

//demi:variant text
type ClientContentText struct {
	Text string `json:"text"`
}

func (ClientContentText) isClientContent() {}

//demi:variant reference
type ClientContentReference struct {
	Reference string `json:"reference"`
}

func (ClientContentReference) isClientContent() {}

// A file the page uploaded (`web-api.md` § Uploads and media); the
// backend writes it to the Host before the session sees the content.
//
//demi:variant upload
type ClientContentUpload struct {
	// The upload's attachment id.
	Ref string `json:"ref" check:"chars=1.."`
	// The name the file is written under: no path separator, no NUL,
	// and neither `.` nor `..`. The pattern has no lookaround, so that
	// the browser's engine and Rust's read it alike.
	FileName string `json:"fileName" check:"chars=1..255,pattern=ClientContentFileNamePattern"`
}

func (ClientContentUpload) isClientContent() {}

// A file on a paired device, read at execution time
// (`web-api.md` § Device files and remote references).
//
//demi:variant remote_file
type ClientContentRemoteFile struct {
	DeviceID string `json:"deviceId" check:"chars=1.."`
	// Absolute.
	Path string `json:"path" check:"chars=1..4096,pattern=ClientContentPathPattern"`
}

func (ClientContentRemoteFile) isClientContent() {}

// In an edit: native media the edited message holds, by blob reference.
//
//demi:variant media
type ClientContentMedia struct {
	Media MediaRef `json:"media"`
}

func (ClientContentMedia) isClientContent() {}

// In an edit: an attachment record of the edited message, by the path
// the record holds (`message-editing.md` § Files the edit keeps).
//
//demi:variant attachment
type ClientContentAttachment struct {
	Path string `json:"path" check:"chars=1..4096"`
}

func (ClientContentAttachment) isClientContent() {}

// Native media an edited message holds, which its edit keeps.
//
//demi:union tag=type
//demi:export
type MediaRef interface{ isMediaRef() }

//demi:variant image
type MediaRefImage struct {
	Ref       core.BlobRef `json:"ref" check:"func=core.Validate"`
	MediaType string       `json:"mediaType"`
}

func (MediaRefImage) isMediaRef() {}

//demi:variant video
type MediaRefVideo struct {
	Ref       core.BlobRef `json:"ref" check:"func=core.Validate"`
	MediaType string       `json:"mediaType"`
}

func (MediaRefVideo) isMediaRef() {}

//demi:variant document
type MediaRefDocument struct {
	Ref       core.BlobRef `json:"ref" check:"func=core.Validate"`
	MediaType string       `json:"mediaType"`
	FileName  string       `json:"fileName" check:"chars=1.."`
}

func (MediaRefDocument) isMediaRef() {}

func (ClientFrameOpen) Kind() ClientFrameKind { return ClientFrameKindOpen }

func (ClientFrameSend) Kind() ClientFrameKind { return ClientFrameKindSend }

func (ClientFrameEditAndSend) Kind() ClientFrameKind { return ClientFrameKindEditAndSend }

func (ClientFrameSteer) Kind() ClientFrameKind { return ClientFrameKindSteer }

func (ClientFrameCancelPendingSteer) Kind() ClientFrameKind { return ClientFrameKindCancelPendingSteer }

func (ClientFrameDequeueMessage) Kind() ClientFrameKind { return ClientFrameKindDequeueMessage }

func (ClientFrameSendQueuedMessage) Kind() ClientFrameKind { return ClientFrameKindSendQueuedMessage }

func (ClientFrameSteerQueuedMessage) Kind() ClientFrameKind { return ClientFrameKindSteerQueuedMessage }

func (ClientFrameClearMessageQueue) Kind() ClientFrameKind { return ClientFrameKindClearMessageQueue }

func (ClientFrameAbort) Kind() ClientFrameKind { return ClientFrameKindAbort }

func (ClientFrameAbortSubagents) Kind() ClientFrameKind { return ClientFrameKindAbortSubagents }

func (ClientFrameAbortSubagent) Kind() ClientFrameKind { return ClientFrameKindAbortSubagent }

func (ClientFrameRetry) Kind() ClientFrameKind { return ClientFrameKindRetry }

func (ClientFrameResume) Kind() ClientFrameKind { return ClientFrameKindResume }

func (ClientFrameCompact) Kind() ClientFrameKind { return ClientFrameKindCompact }

func (ClientFrameShellWrite) Kind() ClientFrameKind { return ClientFrameKindShellWrite }

func (ClientFrameShellAbort) Kind() ClientFrameKind { return ClientFrameKindShellAbort }

func (ClientFrameSyncTranscript) Kind() ClientFrameKind { return ClientFrameKindSyncTranscript }

func (ClientFrameClose) Kind() ClientFrameKind { return ClientFrameKindClose }
