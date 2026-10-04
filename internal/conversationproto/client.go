package conversationproto

import "github.com/wspl/demi/internal/types"

// A frame the web app sends. The connection belongs to one conversation, so
// no frame names a session or a working directory.
// +demi:root direction=send output=protocol
// +demi:union tag=type
//
//sumtype:decl
type clientFrame interface {
	isClientFrame()
	Kind() ClientFrameKind
}

// Attach this connection to the conversation's tree, restoring it when
// it is not live with the model selection the conversation's record
// holds; no frame names a model (`runtime.md` § Model switch).
// +demi:variant clientFrame open
type OpenFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*OpenFrame) Kind() ClientFrameKind { return ClientFrameKindOpen }

// Submit a message; its id becomes its turn's id and its queue entry's.
// +demi:variant clientFrame send
// +demi:check validateSend
type SendFrame struct {
	MessageID types.TurnID    `json:"messageId"`
	Content   []ClientContent `json:"content"`
}

// Kind identifies the command named by a rejected frame.
func (*SendFrame) Kind() ClientFrameKind { return ClientFrameKindSend }

// Replace a user message and everything after it.
// +demi:variant clientFrame edit_and_send
// +demi:check validateEditAndSend
type EditAndSendFrame struct {
	Request EditRequest `json:"request"`
}

// Kind identifies the command named by a rejected frame.
func (*EditAndSendFrame) Kind() ClientFrameKind { return ClientFrameKindEditAndSend }

// Add input to the running turn; its id is its `steer` block's.
// +demi:variant clientFrame steer
// +demi:check validateSteer
type SteerFrame struct {
	SteerID types.BlockID   `json:"steerId"`
	Content []ClientContent `json:"content"`
}

// Kind identifies the command named by a rejected frame.
func (*SteerFrame) Kind() ClientFrameKind { return ClientFrameKindSteer }

// +demi:variant clientFrame cancel_pending_steer
type CancelPendingSteerFrame struct {
	SteerID types.BlockID `json:"steerId"`
}

// Kind identifies the command named by a rejected frame.
func (*CancelPendingSteerFrame) Kind() ClientFrameKind { return ClientFrameKindCancelPendingSteer }

// +demi:variant clientFrame dequeue_message
type DequeueMessageFrame struct {
	MessageID types.TurnID `json:"messageId"`
}

// Kind identifies the command named by a rejected frame.
func (*DequeueMessageFrame) Kind() ClientFrameKind { return ClientFrameKindDequeueMessage }

// Move a queued message to the front, so that it runs next.
// +demi:variant clientFrame send_queued_message
type SendQueuedMessageFrame struct {
	MessageID types.TurnID `json:"messageId"`
}

// Kind identifies the command named by a rejected frame.
func (*SendQueuedMessageFrame) Kind() ClientFrameKind { return ClientFrameKindSendQueuedMessage }

// Turn a queued message into a steer of the running turn.
// +demi:variant clientFrame steer_queued_message
type SteerQueuedMessageFrame struct {
	MessageID types.TurnID  `json:"messageId"`
	SteerID   types.BlockID `json:"steerId"`
}

// Kind identifies the command named by a rejected frame.
func (*SteerQueuedMessageFrame) Kind() ClientFrameKind { return ClientFrameKindSteerQueuedMessage }

// +demi:variant clientFrame clear_message_queue
type ClearMessageQueueFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*ClearMessageQueueFrame) Kind() ClientFrameKind { return ClientFrameKindClearMessageQueue }

// Stop one thing (`runtime.md` § Stop).
// +demi:variant clientFrame abort
type AbortFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*AbortFrame) Kind() ClientFrameKind { return ClientFrameKindAbort }

// +demi:variant clientFrame abort_subagents
type AbortSubagentsFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*AbortSubagentsFrame) Kind() ClientFrameKind { return ClientFrameKindAbortSubagents }

// Stop one live subagent with its subtree.
// +demi:variant clientFrame abort_subagent
type AbortSubagentFrame struct {
	SubagentID types.NodeID `json:"subagentId"`
}

// Kind identifies the command named by a rejected frame.
func (*AbortSubagentFrame) Kind() ClientFrameKind { return ClientFrameKindAbortSubagent }

// +demi:variant clientFrame retry
type RetryFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*RetryFrame) Kind() ClientFrameKind { return ClientFrameKindRetry }

// +demi:variant clientFrame resume
type ResumeFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*ResumeFrame) Kind() ClientFrameKind { return ClientFrameKindResume }

// +demi:variant clientFrame compact
type CompactFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*CompactFrame) Kind() ClientFrameKind { return ClientFrameKindCompact }

// Write stdin to a running command.
// +demi:variant clientFrame shell_write
type ShellWriteFrame struct {
	CommandID types.CommandID `json:"commandId"`
	Stdin     string          `json:"stdin"`
}

// Kind identifies the command named by a rejected frame.
func (*ShellWriteFrame) Kind() ClientFrameKind { return ClientFrameKindShellWrite }

// Stop a running command.
// +demi:variant clientFrame shell_abort
type ShellAbortFrame struct {
	CommandID types.CommandID `json:"commandId"`
}

// Kind identifies the command named by a rejected frame.
func (*ShellAbortFrame) Kind() ClientFrameKind { return ClientFrameKindShellAbort }

// Ask for a fresh transcript, after a gap in the patch revisions.
// +demi:variant clientFrame sync_transcript
type SyncTranscriptFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*SyncTranscriptFrame) Kind() ClientFrameKind { return ClientFrameKindSyncTranscript }

// Dispose the tree.
// +demi:variant clientFrame close
type CloseFrame struct{}

// Kind identifies the command named by a rejected frame.
func (*CloseFrame) Kind() ClientFrameKind { return ClientFrameKindClose }

// Which frame a frame is: its `type`, which a `rejected` frame names.
// +demi:enum open send edit_and_send steer cancel_pending_steer dequeue_message send_queued_message steer_queued_message clear_message_queue abort abort_subagents abort_subagent retry resume compact shell_write shell_abort sync_transcript close
type ClientFrameKind string

// Client frame kinds name the commands the web app can send.
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
type EditRequest struct {
	// Chosen by the web app, so a repeated request is recognized.
	OperationID types.OperationID `json:"operationId"`
	// The `user` block the edit replaces.
	TargetBlockID types.BlockID `json:"targetBlockId"`
	// The transcript version the editor showed; a stale one is refused.
	Version TranscriptVersion `json:"version"`
	// The complete replacement content.
	// +demi:length min=1
	Content []ClientContent `json:"content"`
}

// One part of the content the web app sends. Files are referred to, never
// carried: a new file is an upload or a file on a paired device, and an edit
// keeps what the edited message holds by reference.
// +demi:union tag=type
//
//sumtype:decl
type ClientContent interface {
	isClientContent()
}

// +demi:variant ClientContent text
type TextContent struct {
	Text string `json:"text"`
}

// +demi:variant ClientContent reference
type ReferenceContent struct {
	Reference string `json:"reference"`
}

// A file the page uploaded (`web-api.md` § Uploads and media); the
// backend writes it to the Host before the session sees the content.
// +demi:variant ClientContent upload
type UploadContent struct {
	// The upload's attachment id.
	// +demi:length chars min=1
	Ref string `json:"ref"`
	// The name the file is written under: no path separator, no NUL,
	// and neither `.` nor `..`. The pattern has no lookaround, so that
	// a web browser's regex engine and Go's regexp read it alike.
	// +demi:length chars min=1 max=255
	// +demi:pattern ^(?:[^./\\\x00][^/\\\x00]*|\.[^./\\\x00][^/\\\x00]*|\.\.[^/\\\x00]+)$
	FileName string `json:"fileName"`
}

// A file on a paired device, read at execution time
// (`web-api.md` § Device files and remote references).
// +demi:variant ClientContent remote_file
type RemoteFileContent struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
	// Absolute.
	// +demi:length chars min=1 max=4096
	// +demi:pattern ^/[^\x00]*$
	Path string `json:"path"`
}

// In an edit: native media the edited message holds, by blob reference.
// +demi:variant ClientContent media
type MediaContent struct {
	Media MediaRef `json:"media"`
}

// In an edit: an attachment record of the edited message, by the path
// the record holds (`message-editing.md` § Files the edit keeps).
// +demi:variant ClientContent attachment
type AttachmentContent struct {
	// +demi:length chars min=1 max=4096
	Path string `json:"path"`
}

// Native media an edited message holds, which its edit keeps.
// +demi:union tag=type
//
//sumtype:decl
type MediaRef interface {
	isMediaRef()
}

// +demi:variant MediaRef image
type MediaImageRef struct {
	Ref       types.BlobRef `json:"ref"`
	MediaType string        `json:"mediaType"`
}

// +demi:variant MediaRef video
type MediaVideoRef struct {
	Ref       types.BlobRef `json:"ref"`
	MediaType string        `json:"mediaType"`
}

// +demi:variant MediaRef document
type MediaDocumentRef struct {
	Ref       types.BlobRef `json:"ref"`
	MediaType string        `json:"mediaType"`
	// +demi:length chars min=1
	FileName string `json:"fileName"`
}
