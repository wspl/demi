package core

//go:generate go run ../../tools/contractgen

// A transcript block. A steer's id and an agent message's id are the ids
// of the blocks they become.
// +demi:length min=1
// +demi:id
// +demi:schema-primitive
type BlockID string

// A turn: every block a turn writes carries it. A message's id, which the
// web app chooses, is the id of the turn the message starts.
// +demi:length min=1
// +demi:id
// +demi:schema-primitive
type TurnID string

// An agent node: the root of a conversation or one of its subagents.
// +demi:length min=1
// +demi:id
// +demi:schema-primitive
type NodeID string

// A scheduled yield wakeup.
// +demi:length min=1
// +demi:id
// +demi:schema-primitive
type WakeupID string

// A shell of a node's shell environment.
// +demi:length min=1
// +demi:id
// +demi:schema-primitive
type ShellID string

// A command a shell runs; its handle for `shell_status`, `shell_write`
// and `shell_abort`.
// +demi:length min=1
// +demi:id
// +demi:schema-primitive
type CommandID string

// A blob's name: the SHA-256 of its bytes in lowercase hexadecimal
// (`storage.md` § Encodings and digests). A blob belongs to its owner's
// namespace, so a name grants no access across users.
// +demi:id
// +demi:schema-primitive
// +demi:pattern ^[0-9a-f]{64}$
type BlobRef string

// A moment in whole milliseconds, which JSON writes as
// `2026-09-21T14:13:20.000Z`.
// +demi:timestamp
type Timestamp string

// One block of a transcript.
// +demi:root direction=receive output=protocol
// +demi:union tag=type
//
//sumtype:decl
type Block interface {
	isBlock()
	ID() BlockID
	CreatedAt() Timestamp
	Model() ModelSelection
	IsEditable() bool
}

// A message the user submitted, with the runtime's text for the turn.
// +demi:variant Block user
type UserBlock struct {
	BlockID BlockID `json:"id"`
	// The message's id, which starts the turn.
	TurnID    TurnID         `json:"turnId"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	// The content as submitted.
	Content []UserContentBlock `json:"content"`
	// The runtime's text for the turn, a subagent's identity, which the
	// model receives before the content; null for the root.
	// +demi:nullable
	Preamble *string `json:"preamble"`
}

// What one context source told the node before a request (`runtime.md`
// § Context), which the model receives as a user message.
// +demi:variant Block context
type ContextBlock struct {
	BlockID   BlockID        `json:"id"`
	TurnID    TurnID         `json:"turnId"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	// The source that answered: `execution`, or a plugin's id.
	// +demi:length min=1 max=64
	Source string `json:"source"`
	Text   string `json:"text"`
}

// A fired yield wakeup. The model receives the fixed wakeup text as a user
// message or as a steer, as the placement says.
// +demi:variant Block wakeup
type WakeupBlock struct {
	BlockID   BlockID         `json:"id"`
	TurnID    TurnID          `json:"turnId"`
	Timestamp Timestamp       `json:"createdAt"`
	Selection ModelSelection  `json:"model"`
	Placement WakeupPlacement `json:"placement"`
}

// Where a fired wakeup entered the transcript.
// +demi:enum new_turn steer
type WakeupPlacement string

// A human steer, written at a continuation boundary. Its id is the steer's.
// +demi:variant Block steer
type SteerBlock struct {
	BlockID   BlockID            `json:"id"`
	TurnID    TurnID             `json:"turnId"`
	Timestamp Timestamp          `json:"createdAt"`
	Selection ModelSelection     `json:"model"`
	Content   []UserContentBlock `json:"content"`
}

// A message from another agent of the tree. Its id is the message's.
// +demi:variant Block agent_message
// +demi:check validateAgentMessageBlock
type AgentMessageBlock struct {
	BlockID   BlockID        `json:"id"`
	TurnID    TurnID         `json:"turnId"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	Message   AgentMessage   `json:"message"`
}

// The turn continues after a cut; the model receives "Continue from where
// you left off."
// +demi:variant Block resume
type ResumeBlock struct {
	BlockID   BlockID        `json:"id"`
	TurnID    TurnID         `json:"turnId"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
}

// The stopped marker. The user sees it until the turn is continued.
// +demi:variant Block abort
type AbortBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	// Set once the stopped turn is continued.
	IsResumed bool `json:"isResumed"`
}

// Reasoning text, with the vendor's signature when it signed it. A signed
// block is replayed whole.
// +demi:variant Block thinking
type ThinkingBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	Text      string         `json:"text"`
	// +demi:nullable
	Signature *string `json:"signature"`
}

// Opaque reasoning data, replayed whole and never shown.
// +demi:variant Block redacted_thinking
type RedactedThinkingBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	Data      string         `json:"data"`
}

// Assistant text.
// +demi:variant Block text
type TextBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	Text      string         `json:"text"`
	// Set once the text is complete and a Fork may start after it
	// (`conversation-fork.md`); omitted until then.
	Forkable bool `json:"forkable,omitempty"`
}

// A tool call the provider requested, completed by the session with its
// result.
// +demi:variant Block tool_call
type ToolCallBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
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

// Where a tool call is.
// +demi:enum executing completed error
type ToolCallStatus string

// The usage of one completed provider request, which anchors the context
// estimate.
// +demi:variant Block response
type ResponseBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	Usage     TokenUsage     `json:"usage"`
}

// A failed request or an interrupted turn (`failures-and-recovery.md`
// § The failure record).
// +demi:variant Block error
type ErrorBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	Message   string         `json:"message"`
	// The failure's code, such as `rate_limit` or `interrupted`; null when
	// it has none.
	// +demi:nullable
	Code *string `json:"code"`
	// The provider's record of the failure, when a provider failed.
	Diagnostics *ProviderErrorDiagnostics `json:"diagnostics,omitempty"`
}

// Compaction's summary of the history before it, inserted where the kept
// history begins.
// +demi:variant Block compaction_boundary
type CompactionBoundaryBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	Summary   string         `json:"summary"`
	// +demi:range max=9007199254740991
	SummaryTokens uint64 `json:"summaryTokens"`
}

// Compaction's estimate of the size of what it summarized.
// +demi:variant Block compaction_marker
type CompactionMarkerBlock struct {
	BlockID   BlockID        `json:"id"`
	Timestamp Timestamp      `json:"createdAt"`
	Selection ModelSelection `json:"model"`
	// The boundary this pass inserted.
	BoundaryID BlockID `json:"boundaryId"`
	// +demi:range max=9007199254740991
	CompactedTokens uint64 `json:"compactedTokens"`
}

// One part of a message or a steer, in the transcript and in the queue.
// +demi:union tag=type
//
//sumtype:decl
type UserContentBlock interface{ isUserContentBlock() }

// +demi:variant UserContentBlock text
type UserText struct {
	Text string `json:"text"`
}

// An image the model reads natively.
// +demi:variant UserContentBlock image
type UserImage struct {
	Source MediaSource `json:"source"`
}

// A video the model reads natively; only a model whose catalog marks
// video support receives one.
// +demi:variant UserContentBlock video
type UserVideo struct {
	Source MediaSource `json:"source"`
}

// A PDF the model reads natively.
// +demi:variant UserContentBlock document
type UserDocument struct {
	Source DocumentSource `json:"source"`
}

// Text that names something, such as a file on a paired device and the
// command that reads it; the model reads it as text.
// +demi:variant UserContentBlock reference
type UserReference struct {
	Reference string `json:"reference"`
}

// The record of a file that came with a message: on the conversation's Host
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

// Where a message's image or video is: never its bytes, which a session
// holds beside the transcript while a request can send them (`runtime.md`
// § Media).
// +demi:union tag=type
//
//sumtype:decl
type MediaSource interface{ isMediaSource() }

// A URL the provider fetches.
// +demi:variant MediaSource url
type MediaURL struct {
	URL string `json:"url"`
}

// The bytes in the conversation owner's blob namespace, as a store keeps
// them and as the page receives them.
// +demi:variant MediaSource ref
type MediaSourceRef struct {
	Ref       BlobRef `json:"ref"`
	MediaType string  `json:"mediaType"`
}

// Where a document's bytes are, with the name the file came with: in the
// conversation owner's blob namespace. A tagged enum of one variant, so a
// stored document keeps `{ "type": "ref", ... }`.
// +demi:union tag=type
//
//sumtype:decl
type DocumentSource interface{ isDocumentSource() }

// +demi:variant DocumentSource ref
type DocumentRef struct {
	Ref       BlobRef `json:"ref"`
	MediaType string  `json:"mediaType"`
	FileName  string  `json:"fileName"`
}

// One part of a tool's result.
// +demi:union tag=type
//
//sumtype:decl
type ToolResultContentBlock interface{ isToolResultContentBlock() }

// +demi:variant ToolResultContentBlock text
type ToolText struct {
	Text string `json:"text"`
}

// +demi:variant ToolResultContentBlock image
type ToolImage struct {
	Source ToolMediaSource `json:"source"`
}

// +demi:variant ToolResultContentBlock video
type ToolVideo struct {
	Source ToolMediaSource `json:"source"`
}

// An image or a video the result no longer holds, in its place: what
// it was and why it is gone (`runtime.md` § Media). The model reads it
// as one line of text.
// +demi:variant ToolResultContentBlock gone
type ToolGone struct {
	Kind      ModelMediaKind `json:"kind"`
	MediaType string         `json:"mediaType"`
	Cause     GoneCause      `json:"cause"`
}

// Whether a model reads a medium as an image or as a video.
// +demi:enum image video
type ModelMediaKind string

// Why a tool result's image or video is gone.
// +demi:union tag=type
//
//sumtype:decl
type GoneCause interface{ isGoneCause() }

// Its bytes could not be stored when the result entered the
// transcript; `error` is the store's.
// +demi:variant GoneCause not_stored
type NotStored struct {
	Error string `json:"error"`
}

// It was retired at `at`, 30 days on (`runtime.md` § Retired tool
// media).
// +demi:variant GoneCause retired
type Retired struct {
	At Timestamp `json:"at"`
}

// Where the bytes of a tool result's image or video are: in the
// conversation owner's blob namespace. A tagged enum of one variant, so a
// stored result keeps `{ "type": "ref", ... }`.
// +demi:union tag=type
//
//sumtype:decl
type ToolMediaSource interface{ isToolMediaSource() }

// +demi:variant ToolMediaSource ref
type ToolMediaRef struct {
	Ref       BlobRef `json:"ref"`
	MediaType string  `json:"mediaType"`
}

// A file that came with the message.
// +demi:variant UserContentBlock attachment
type UserAttachment struct{ Attachment }
