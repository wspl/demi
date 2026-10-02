package core

//go:generate go run ../../tools/contractgen

// BlockID is a transcript block. A steer's id and an agent message's id are the ids
// of the blocks they become.
// +demi:length min=1
// +demi:id
type BlockID string

// TurnID is a turn: every block a turn writes carries it. A message's id, which the
// web app chooses, is the id of the turn the message starts.
// +demi:length min=1
// +demi:id
type TurnID string

// NodeID is an agent node: the root of a conversation or one of its subagents.
// +demi:length min=1
// +demi:id
type NodeID string

// WakeupID is a scheduled yield wakeup.
// +demi:length min=1
// +demi:id
type WakeupID string

// ShellID is a shell of a node's shell environment.
// +demi:length min=1
// +demi:id
type ShellID string

// CommandID is a command a shell runs; its handle for shell_status, shell_write and shell_abort.
// +demi:length min=1
// +demi:id
type CommandID string

// BlobRef is a blob's name: the SHA-256 of its bytes in lowercase hexadecimal
// (`storage.md` § Encodings and digests). A blob belongs to its owner's
// namespace, so a name grants no access across users.
// +demi:id
// +demi:pattern ^[0-9a-f]{64}$
type BlobRef string

// Timestamp is a moment in whole milliseconds, which JSON writes as
// `2026-09-21T14:13:20.000Z`.
// +demi:timestamp
type Timestamp string

// Block is one block of a transcript.
// +demi:root direction=receive output=protocol
// +demi:union tag=type
//
//sumtype:decl
type Block interface{ isBlock() }

// UserBlock is a message the user submitted, with the runtime's text for the turn.
// +demi:variant Block user
type UserBlock struct {
	ID BlockID `json:"id"`
	// The message's id, which starts the turn.
	TurnID    TurnID         `json:"turnId"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	// The content as submitted.
	Content []UserContentBlock `json:"content"`
	// The runtime's text for the turn, a subagent's identity, which the
	// model receives before the content; null for the root.
	// +demi:nullable
	Preamble *string `json:"preamble"`
}

// ContextBlock represents what one context source told the node before a request (`runtime.md`
// § Context), which the model receives as a user message.
// +demi:variant Block context
type ContextBlock struct {
	ID        BlockID        `json:"id"`
	TurnID    TurnID         `json:"turnId"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	// The source that answered: `execution`, or a plugin's id.
	// +demi:length min=1 max=64
	Source string `json:"source"`
	Text   string `json:"text"`
}

// WakeupBlock is a fired yield wakeup. The model receives the fixed wakeup text as a user
// message or as a steer, as the placement says.
// +demi:variant Block wakeup
type WakeupBlock struct {
	ID        BlockID         `json:"id"`
	TurnID    TurnID          `json:"turnId"`
	CreatedAt Timestamp       `json:"createdAt"`
	Model     ModelSelection  `json:"model"`
	Placement WakeupPlacement `json:"placement"`
}

// WakeupPlacement describes where a fired wakeup entered the transcript.
// +demi:enum new_turn steer
type WakeupPlacement string

// SteerBlock is a human steer, written at a continuation boundary. Its id is the steer's.
// +demi:variant Block steer
type SteerBlock struct {
	ID        BlockID            `json:"id"`
	TurnID    TurnID             `json:"turnId"`
	CreatedAt Timestamp          `json:"createdAt"`
	Model     ModelSelection     `json:"model"`
	Content   []UserContentBlock `json:"content"`
}

// AgentMessageBlock is a message from another agent of the tree. Its id is the message's.
// +demi:variant Block agent_message
// +demi:check validateAgentMessageBlock
type AgentMessageBlock struct {
	ID        BlockID        `json:"id"`
	TurnID    TurnID         `json:"turnId"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	Message   AgentMessage   `json:"message"`
}

// ResumeBlock is the turn continues after a cut; the model receives "Continue from where
// you left off."
// +demi:variant Block resume
type ResumeBlock struct {
	ID        BlockID        `json:"id"`
	TurnID    TurnID         `json:"turnId"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
}

// AbortBlock is the stopped marker. The user sees it until the turn is continued.
// +demi:variant Block abort
type AbortBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	// Set once the stopped turn is continued.
	IsResumed bool `json:"isResumed"`
}

// ThinkingBlock represents reasoning text, with the vendor's signature when it signed it. A signed
// block is replayed whole.
// +demi:variant Block thinking
type ThinkingBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
	// +demi:nullable
	Signature *string `json:"signature"`
}

// RedactedThinkingBlock represents opaque reasoning data, replayed whole and never shown.
// +demi:variant Block redacted_thinking
type RedactedThinkingBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	Data      string         `json:"data"`
}

// TextBlock represents assistant text.
// +demi:variant Block text
type TextBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
	// Set once the text is complete and a Fork may start after it
	// (`conversation-fork.md`); omitted until then.
	Forkable bool `json:"forkable,omitempty"`
}

// ToolCallBlock is a tool call the provider requested, completed by the session with its
// result.
// +demi:variant Block tool_call
type ToolCallBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	// The provider's id of the call.
	ToolUseID string `json:"toolUseId"`
	ToolName  string `json:"toolName"`
	// The call's input as the JSON text the provider supplied, which need
	// not be valid JSON.
	Input  string         `json:"input"`
	Status ToolCallStatus `json:"status"`
	// The result, once the call completed.
	Output []ToolResultContentBlock `json:"output"`
	// +demi:nullable
	View ToolView `json:"view"`
}

// ToolCallStatus describes where a tool call is.
// +demi:enum executing completed error
type ToolCallStatus string

// ResponseBlock is the usage of one completed provider request, which anchors the context
// estimate.
// +demi:variant Block response
type ResponseBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	Usage     TokenUsage     `json:"usage"`
}

// ErrorBlock is a failed request or an interrupted turn (`failures-and-recovery.md`
// § The failure record).
// +demi:variant Block error
type ErrorBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	Message   string         `json:"message"`
	// The failure's code, such as `rate_limit` or `interrupted`; null when
	// it has none.
	// +demi:nullable
	Code *string `json:"code"`
	// The provider's record of the failure, when a provider failed.
	Diagnostics *ProviderErrorDiagnostics `json:"diagnostics,omitempty"`
}

// CompactionBoundaryBlock represents compaction's summary of the history before it, inserted where the kept
// history begins.
// +demi:variant Block compaction_boundary
type CompactionBoundaryBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	Summary   string         `json:"summary"`
	// +demi:range max=9007199254740991
	SummaryTokens uint64 `json:"summaryTokens"`
}

// CompactionMarkerBlock represents compaction's estimate of the size of what it summarized.
// +demi:variant Block compaction_marker
type CompactionMarkerBlock struct {
	ID        BlockID        `json:"id"`
	CreatedAt Timestamp      `json:"createdAt"`
	Model     ModelSelection `json:"model"`
	// The boundary this pass inserted.
	BoundaryID BlockID `json:"boundaryId"`
	// +demi:range max=9007199254740991
	CompactedTokens uint64 `json:"compactedTokens"`
}

// UserContentBlock is one part of a message or a steer, in the transcript and in the queue.
// +demi:union tag=type
//
//sumtype:decl
type UserContentBlock interface{ isUserContentBlock() }

// UserText is a transcript contract value.
// +demi:variant UserContentBlock text
type UserText struct {
	Text string `json:"text"`
}

// UserImage is an image the model reads natively.
// +demi:variant UserContentBlock image
type UserImage struct {
	Source MediaSource `json:"source"`
}

// UserVideo is a video the model reads natively; only a model whose catalog marks video support receives one.
// +demi:variant UserContentBlock video
type UserVideo struct {
	Source MediaSource `json:"source"`
}

// UserDocument is a PDF the model reads natively.
// +demi:variant UserContentBlock document
type UserDocument struct {
	Source DocumentSource `json:"source"`
}

// UserReference represents text that names something, such as a file on a paired device and the command that reads it; the model reads it as text.
// +demi:variant UserContentBlock reference
type UserReference struct {
	Reference string `json:"reference"`
}

// Attachment is the record of a file that came with a message: on the conversation's Host
// at `path`, never inlined. The model reads it as a tag that names the file
// ([`attachment_tag`]); the page draws the file's tile from it.
type Attachment struct {
	// +demi:length min=1
	Name string `json:"name"`
	// Absolute, on the conversation's Host.
	// +demi:length min=1
	Path      string `json:"path"`
	MediaType string `json:"mediaType"`
	// +demi:range max=9007199254740991
	SizeBytes uint64 `json:"sizeBytes"`
	// The uploaded bytes in the blob store, from which the page fetches
	// them.
	SHA256 BlobRef `json:"sha256"`
	// The opening of a text file, for the tile that shows it as a page.
	Snippet *string `json:"snippet,omitempty"`
}

// MediaSource describes where a message's image or video is: never its bytes, which a session
// holds beside the transcript while a request can send them (`runtime.md`
// § Media).
// +demi:union tag=type
//
//sumtype:decl
type MediaSource interface{ isMediaSource() }

// MediaURL is a URL the provider fetches.
// +demi:variant MediaSource url
type MediaURL struct {
	URL string `json:"url"`
}

// MediaSourceRef is the bytes in the conversation owner's blob namespace, as a store keeps
// them and as the page receives them.
// +demi:variant MediaSource ref
type MediaSourceRef struct {
	Ref       BlobRef `json:"ref"`
	MediaType string  `json:"mediaType"`
}

// DocumentSource describes where a document's bytes are, with the name the file came with: in the
// conversation owner's blob namespace. A tagged enum of one variant, so a
// stored document keeps `{ "type": "ref", ... }`.
// +demi:union tag=type
//
//sumtype:decl
type DocumentSource interface{ isDocumentSource() }

// DocumentRef is a transcript contract value.
// +demi:variant DocumentSource ref
type DocumentRef struct {
	Ref       BlobRef `json:"ref"`
	MediaType string  `json:"mediaType"`
	FileName  string  `json:"fileName"`
}

// ToolResultContentBlock is one part of a tool's result.
// +demi:union tag=type
//
//sumtype:decl
type ToolResultContentBlock interface{ isToolResultContentBlock() }

// ToolText is a transcript contract value.
// +demi:variant ToolResultContentBlock text
type ToolText struct {
	Text string `json:"text"`
}

// ToolImage is a transcript contract value.
// +demi:variant ToolResultContentBlock image
type ToolImage struct {
	Source ToolMediaSource `json:"source"`
}

// ToolVideo is a transcript contract value.
// +demi:variant ToolResultContentBlock video
type ToolVideo struct {
	Source ToolMediaSource `json:"source"`
}

// ToolGone is an image or a video the result no longer holds, in its place: what
// it was and why it is gone (runtime.md § Media). The model reads it as one line of text.
// +demi:variant ToolResultContentBlock gone
type ToolGone struct {
	Kind      ModelMediaKind `json:"kind"`
	MediaType string         `json:"mediaType"`
	Cause     GoneCause      `json:"cause"`
}

// ModelMediaKind records whether a model reads a medium as an image or as a video.
// +demi:enum image video
type ModelMediaKind string

// GoneCause represents why a tool result's image or video is gone.
// +demi:union tag=type
//
//sumtype:decl
type GoneCause interface{ isGoneCause() }

// NotStored represents its bytes could not be stored when the result entered the transcript; error is the store's.
// +demi:variant GoneCause not_stored
type NotStored struct {
	Error string `json:"error"`
}

// Retired represents it was retired at at, 30 days on (runtime.md § Retired tool media).
// +demi:variant GoneCause retired
type Retired struct {
	At Timestamp `json:"at"`
}

// ToolMediaSource describes where the bytes of a tool result's image or video are: in the
// conversation owner's blob namespace. A tagged enum of one variant, so a
// stored result keeps `{ "type": "ref", ... }`.
// +demi:union tag=type
//
//sumtype:decl
type ToolMediaSource interface{ isToolMediaSource() }

// ToolMediaRef is a transcript contract value.
// +demi:variant ToolMediaSource ref
type ToolMediaRef struct {
	Ref       BlobRef `json:"ref"`
	MediaType string  `json:"mediaType"`
}

// UserAttachment wraps an attachment as tagged user content.
// +demi:variant UserContentBlock attachment
type UserAttachment struct{ Attachment }
