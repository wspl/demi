package webapi

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// The most items one batch changes.
const BatchMax = 100

// The most characters (Unicode scalar values) a conversation's title has.
const TitleMax = 256

// Where a conversation's work runs (`sessions-and-targets.md` § Resolve a
// target): the user's Cloud, a directory on a paired device, or a
// workspace. A new conversation runs on the Cloud, in its session
// directory.
// +demi:union tag=kind
//
//sumtype:decl
type ConversationTarget interface{ conversationTarget() }

// +demi:variant ConversationTarget cloud
type ConversationTargetCloud struct {
	// An absolute directory on the Cloud; the conversation's session
	// directory without it.
	// +demi:length chars min=1
	// +demi:pattern ^/
	Path *string `json:"path,omitempty"`
}

// +demi:variant ConversationTarget device
type ConversationTargetDevice struct {
	DeviceID DeviceID `json:"deviceId"`
	// +demi:length chars min=1
	Path string `json:"path"`
}

// +demi:variant ConversationTarget workspace
type ConversationTargetWorkspace struct {
	WorkspaceID WorkspaceID `json:"workspaceId"`
}

// `POST /conversations`: the id the web app chose for a new conversation.
// +demi:root direction=send output=web
type CreateConversation struct {
	ID ConversationID `json:"id"`
}

// A conversation as the web app lists it: its record, with the directory
// its work runs in, its status and its output revision. What the backend
// keeps for itself, such as the owner, stays out.
// +demi:tolerant
type ConversationSummary struct {
	ID       ConversationID `json:"id"`
	Title    string         `json:"title"`
	Archived bool           `json:"archived"`
	Pinned   bool           `json:"pinned"`
	// The output revision the user last acknowledged.
	// +demi:range max=9007199254740991
	ReadRevision uint64             `json:"readRevision"`
	Target       ConversationTarget `json:"target"`
	// Advances with every change of the conversation's execution context,
	// such as a target switch.
	// +demi:range max=9007199254740991
	ContextVersion uint64 `json:"contextVersion"`
	// The conversation's model settings, the value every page shows; null
	// while the conversation has no model yet.
	// +demi:nullable
	Model     *ModelSettings `json:"model"`
	CreatedAt core.Timestamp `json:"createdAt"`
	UpdatedAt core.Timestamp `json:"updatedAt"`
	// The directory the conversation's work runs in, resolved by the backend
	// for every kind of target, so the web app never derives it.
	Cwd    string             `json:"cwd"`
	Status ConversationStatus `json:"status"`
	// The output revision: it advances with each saved change of output,
	// never with the user's input alone.
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
	// Whether output is newer than the read revision.
	Unread bool `json:"unread"`
	// Whether the title has read every message the user sent, so asking
	// for a new one could say nothing the last did not
	// (`product.md` § Conversation titles).
	TitleCurrent bool `json:"titleCurrent"`
	// Whether a title request of the conversation is in flight.
	TitleGenerating bool `json:"titleGenerating"`
	// Whether the conversation's tree is open with commands or profiles of
	// plugins the user has since turned on or off, so a reload would
	// change them (`plugins.md` § A user's plugins).
	PluginsChanged bool `json:"pluginsChanged"`
	// The revision of the conversation's draft, 0 before its first save
	// (`web-api.md` § Conversation drafts): a page reads the draft only when
	// this is higher than the revision it holds.
	// +demi:range max=9007199254740991
	DraftRevision uint64 `json:"draftRevision"`
	// The revision of each plugin's conversation state, in registration
	// order (`web-api.md` § Conversation state of plugins): a page reads a
	// state only when its revision is higher than the one it holds.
	PluginRevisions []PluginRevision `json:"pluginRevisions"`
	// How many of the conversation's jobs ended since the backend started:
	// a page lists the working tree again when it changes (`web-api.md`
	// § File text and working tree changes).
	// +demi:range max=9007199254740991
	WorkingTreeRevision uint64 `json:"workingTreeRevision"`
}

// The revision of one plugin's state for a conversation.
type PluginRevision struct {
	Plugin string `json:"plugin"`
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
}

// A conversation's model settings (`models.md` § A conversation's model
// settings): the provider entry and model, the thinking effort and the
// service tier. A new conversation starts with the user's last choice, which
// is model settings too.
type ModelSettings struct {
	ProviderID ProviderID `json:"providerId"`
	// +demi:length chars min=1
	ModelID string `json:"modelId"`
	// An effort the model lists, `disabled` for thinking off, or null for
	// the model's default.
	// +demi:nullable
	ThinkingEffort *string `json:"thinkingEffort"`
	// A tier the model lists, or null for the vendor's default.
	// +demi:nullable
	ServiceTierID *string `json:"serviceTierId"`
}

// Where a conversation stands (`web-api.md` § Sidebar mutations, read state
// and page synchronization): running or compacting while its live tree
// works, interrupted when its last checkpoint was saved in a turn and no
// session is live, otherwise what its latest terminal block says, or idle.
// +demi:enum running compacting interrupted error stopped completed idle
type ConversationStatus string

// Values of the preceding enumeration.
const (
	ConversationStatusRunning     ConversationStatus = "running"
	ConversationStatusCompacting  ConversationStatus = "compacting"
	ConversationStatusInterrupted ConversationStatus = "interrupted"
	ConversationStatusError       ConversationStatus = "error"
	ConversationStatusStopped     ConversationStatus = "stopped"
	ConversationStatusCompleted   ConversationStatus = "completed"
	ConversationStatusIdle        ConversationStatus = "idle"
)

// `{ conversation }`: the answer of a create.
// +demi:root direction=receive output=web
// +demi:tolerant
type ConversationAnswer struct {
	Conversation ConversationSummary `json:"conversation"`
}

// `GET /conversations`: the caller's conversations in sidebar order,
// pinned first.
// +demi:root direction=receive output=web
// +demi:tolerant
type Conversations struct {
	Conversations []ConversationSummary `json:"conversations"`
}

// `?archived=true|false`: the archived conversations, or the others.
// +demi:root
// +demi:tolerant
type ConversationsQuery struct {
	Archived StrictBool `json:"archived,omitempty"`
}

// `POST /conversations/:id/read`: the output revision the page showed.
// +demi:root direction=send output=web
type ReadRequest struct {
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
}

// `GET /conversations/:id/transcript`: the conversation's history as its
// database holds it, the root's blocks and each subagent's, with the failure
// facts of their error blocks (`backend.md` § Failure facts). Media travels
// by blob reference.
// +demi:root direction=receive output=web
// +demi:tolerant
type Transcript struct {
	Blocks []core.Block `json:"blocks"`
	// The facts of the root's error blocks, by block id; absent when none
	// yields one.
	Failures *framewire.Failures `json:"failures,omitempty"`
	// Every subagent of the tree, in spawn order under each parent.
	Subagents []SubagentHistory `json:"subagents"`
}

// One subagent's history.
// +demi:tolerant
type SubagentHistory struct {
	Subagent framewire.SubagentJob `json:"subagent"`
	Blocks   []core.Block          `json:"blocks"`
	Failures *framewire.Failures   `json:"failures,omitempty"`
}

// `PATCH /conversations/:id`: the fields to change, each applied on its
// own; an absent field stays as it is.
// +demi:root direction=send output=web
type ConversationPatch struct {
	// A rename: 1 to 256 characters after trimming.
	// +demi:length chars min=1 max=256
	Title *Trimmed `json:"title,omitempty"`
	// Archive, or restore.
	Archived *bool `json:"archived,omitempty"`
	Pinned   *bool `json:"pinned,omitempty"`
	// A switch to this model, with the effort and the tier this patch
	// names and the model's defaults for a part it leaves out.
	Model *ModelChoice `json:"model,omitempty"`
	// The conversation's thinking effort: one its model lists, `disabled`
	// for thinking off, or null for the model's default.
	// +demi:length chars min=1
	ThinkingEffort **string `json:"thinkingEffort,omitempty"`
	// The conversation's service tier: one its model lists, or null for the
	// vendor's default.
	// +demi:length chars min=1
	ServiceTierID **string `json:"serviceTierId,omitempty"`
	// A switch of the conversation's execution target.
	Target *ConversationTarget `json:"target,omitempty"`
}

// A provider entry of the user's scope and one of its models.
type ModelChoice struct {
	ProviderID ProviderID `json:"providerId"`
	// +demi:length chars min=1
	ModelID string `json:"modelId"`
}

// A field of a conversation patch, as its result names it.
// +demi:enum title archived pinned model thinking_effort service_tier_id target
type PatchField string

// Values of the preceding enumeration.
const (
	PatchFieldTitle          PatchField = "title"
	PatchFieldArchived       PatchField = "archived"
	PatchFieldPinned         PatchField = "pinned"
	PatchFieldModel          PatchField = "model"
	PatchFieldThinkingEffort PatchField = "thinking_effort"
	PatchFieldServiceTierID  PatchField = "service_tier_id"
	PatchFieldTarget         PatchField = "target"
)

// How one field of a patch went: applied, or refused with the code and
// the HTTP status it would answer alone.
// +demi:union tag=status
//
//sumtype:decl
type FieldResult interface{ fieldResult() }

// +demi:variant FieldResult applied
// +demi:tolerant
type FieldResultApplied struct {
	Field PatchField `json:"field"`
}

// +demi:variant FieldResult failed
// +demi:tolerant
type FieldResultFailed struct {
	Field      PatchField `json:"field"`
	Code       ErrorCode  `json:"code"`
	Message    string     `json:"message"`
	HTTPStatus uint16     `json:"httpStatus"`
}

// The answer of a patch: the conversation as it is now, and each field's
// result in the order they were applied, the archive first.
// +demi:root direction=receive output=web
// +demi:tolerant
type ConversationUpdate struct {
	Conversation ConversationSummary `json:"conversation"`
	Results      []FieldResult       `json:"results"`
}

// `POST /conversations/batch`: up to 100 patches, each of one of the
// caller's conversations.
// +demi:root direction=send output=web
type ConversationBatch struct {
	// +demi:length min=1 max=100
	Items []BatchItem `json:"items"`
}

type BatchItem struct { //nolint:revive // The Rust contract has no product doc comment.
	ID    ConversationID    `json:"id"`
	Patch ConversationPatch `json:"patch"`
}

// The answer of a batch: an outcome per item, in the order given.
// +demi:root direction=receive output=web
// +demi:tolerant
type BatchAnswer struct {
	Results []BatchResult `json:"results"`
}

// One item's outcome: its patch's answer, or why the item was refused,
// such as a conversation the caller does not have.
// +demi:union tag=status
//
//sumtype:decl
type BatchResult interface{ batchResult() }

// +demi:variant BatchResult updated
// +demi:tolerant
type BatchResultUpdated struct {
	ID           ConversationID      `json:"id"`
	Conversation ConversationSummary `json:"conversation"`
	Results      []FieldResult       `json:"results"`
}

// +demi:variant BatchResult refused
// +demi:tolerant
type BatchResultRefused struct {
	ID      ConversationID `json:"id"`
	Code    ErrorCode      `json:"code"`
	Message string         `json:"message"`
}

// `POST /conversations/:id/fork`: the new conversation's id, chosen by the
// web app, and the completed assistant text the history is kept through.
// +demi:root direction=send output=web
type ForkRequest struct {
	ID      ConversationID `json:"id"`
	BlockID core.BlockID   `json:"blockId"`
}

// The answer of a Fork: the new conversation, whose model settings are the
// ones it inherited from its source.
// +demi:root direction=receive output=web
// +demi:tolerant
type ForkAnswer struct {
	Conversation ConversationSummary `json:"conversation"`
}
