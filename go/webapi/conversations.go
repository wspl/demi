package webapi

import (
	"github.com/wspl/demi/go/agentproto"
	"github.com/wspl/demi/go/core"
)

// Where a conversation's work runs (`sessions-and-targets.md` § Resolve a
// target): the user's Cloud, a directory on a paired device, or a
// workspace. A new conversation runs on the Cloud, in its session
// directory.
//
//demi:union tag=kind
//demi:export
type ConversationTarget interface{ isConversationTarget() }

//demi:variant cloud
type ConversationTargetCloud struct {
	// An absolute directory on the Cloud; the conversation's session
	// directory without it.
	Path *string `json:"path,omitzero" check:"bytes=1..,pattern=ConversationTargetPathPattern"`
}

func (ConversationTargetCloud) isConversationTarget() {}

//demi:variant device
type ConversationTargetDevice struct {
	DeviceID DeviceID `json:"deviceId" check:"func=Validate"`
	Path     string   `json:"path" check:"bytes=1.."`
}

func (ConversationTargetDevice) isConversationTarget() {}

//demi:variant workspace
type ConversationTargetWorkspace struct {
	WorkspaceID WorkspaceID `json:"workspaceId" check:"func=Validate"`
}

func (ConversationTargetWorkspace) isConversationTarget() {}

// `POST /conversations`: the id the browser chose for a new conversation.
//
//demi:wire
type CreateConversation struct {
	ID ConversationID `json:"id" check:"func=Validate"`
}

// A conversation as the browser lists it: its record, with the directory
// its work runs in, its status and its output revision. What the backend
// keeps for itself, such as the owner, stays out.
//
//demi:wire open
type ConversationSummary struct {
	ID       ConversationID `json:"id" check:"func=Validate"`
	Title    string         `json:"title"`
	Archived bool           `json:"archived"`
	Pinned   bool           `json:"pinned"`
	// The output revision the user last acknowledged.
	ReadRevision uint64             `json:"readRevision" check:"range=..9007199254740991"`
	Target       ConversationTarget `json:"target"`
	// Advances with every change of the conversation's execution context,
	// such as a target switch.
	ContextVersion uint64 `json:"contextVersion" check:"range=..9007199254740991"`
	// The conversation's model settings, the value every page shows; null
	// while the conversation has no model yet.
	Model     *ModelSettings `json:"model" check:"nullable"`
	CreatedAt core.Timestamp `json:"createdAt" check:"func=core.Validate"`
	UpdatedAt core.Timestamp `json:"updatedAt" check:"func=core.Validate"`
	// The directory the conversation's work runs in, resolved by the backend
	// for every kind of target, so the browser never derives it.
	Cwd    string             `json:"cwd"`
	Status ConversationStatus `json:"status"`
	// The output revision: it advances with each saved change of output,
	// never with the user's input alone.
	Revision uint64 `json:"revision" check:"range=..9007199254740991"`
	// Whether output is newer than the read revision.
	Unread bool `json:"unread"`
	// Whether the title has read every message the user sent, so asking
	// for a new one could say nothing the last did not
	// (`product.md` § Conversation titles).
	TitleCurrent bool `json:"titleCurrent"`
	// Whether a title request of the conversation is in flight.
	TitleGenerating bool `json:"titleGenerating"`
	// The revision of the conversation's draft, 0 before its first save
	// (`web-api.md` § Conversation drafts): a page reads the draft only when
	// this is higher than the revision it holds.
	DraftRevision uint64 `json:"draftRevision" check:"range=..9007199254740991"`
}

// A conversation's model settings (`models.md` § A conversation's model
// settings): the provider entry and model, the thinking effort and the
// service tier. A new conversation starts with the user's last choice, which
// is model settings too.
//
//demi:wire
type ModelSettings struct {
	ProviderID ProviderID `json:"providerId" check:"func=Validate"`
	ModelID    string     `json:"modelId" check:"chars=1.."`
	// An effort the model lists, `disabled` for thinking off, or null for
	// the model's default.
	ThinkingEffort *string `json:"thinkingEffort" check:"nullable"`
	// A tier the model lists, or null for the vendor's default.
	ServiceTierID *string `json:"serviceTierId" check:"nullable"`
}

// Where a conversation stands (`web-api.md` § Sidebar mutations, read state
// and page synchronization): running or compacting while its live tree
// works, interrupted when its last checkpoint was saved in a turn and no
// session is live, otherwise what its latest terminal block says, or idle.
//
//demi:enum
//demi:export
type ConversationStatus string

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
//
//demi:wire open
type ConversationAnswer struct {
	Conversation ConversationSummary `json:"conversation"`
}

// `GET /conversations`: the caller's conversations in sidebar order,
// pinned first.
//
//demi:wire open
type Conversations struct {
	Conversations []ConversationSummary `json:"conversations"`
}

// `POST /conversations/:id/read`: the output revision the page showed.
//
//demi:wire
type ReadRequest struct {
	Revision uint64 `json:"revision" check:"range=..9007199254740991"`
}

// `GET /conversations/:id/transcript`: the conversation's history as its
// database holds it, the root's blocks and each subagent's, with the failure
// facts of their error blocks (`backend.md` § Failure facts). Media travels
// by blob reference.
//
//demi:wire open
type Transcript struct {
	Blocks []core.Block `json:"blocks" check:"each(func=core.Validate)"`
	// The facts of the root's error blocks, by block id; absent when none
	// yields one.
	Failures *map[string]core.ProviderFailureFacts `json:"failures,omitzero" check:"keys(chars=1..),each(func=core.Validate)"`
	// Every subagent of the tree, in spawn order under each parent.
	Subagents []SubagentHistory `json:"subagents"`
}

// One subagent's history.
//
//demi:wire open
type SubagentHistory struct {
	Subagent agentproto.SubagentJob                `json:"subagent" check:"func=agentproto.Validate"`
	Blocks   []core.Block                          `json:"blocks" check:"each(func=core.Validate)"`
	Failures *map[string]core.ProviderFailureFacts `json:"failures,omitzero" check:"keys(chars=1..),each(func=core.Validate)"`
}

// `PATCH /conversations/:id`: the fields to change, each applied on its
// own; an absent field stays as it is.
//
//demi:wire
type ConversationPatch struct {
	// A rename: 1 to 256 characters after trimming.
	Title *Trimmed `json:"title,omitzero" check:"chars=1..256,func=Validate"`
	// Archive, or restore.
	Archived *bool `json:"archived,omitzero"`
	Pinned   *bool `json:"pinned,omitzero"`
	// A switch to this model, with the effort and the tier this patch
	// names and the model's defaults for a part it leaves out.
	Model *ModelChoice `json:"model,omitzero"`
	// The conversation's thinking effort: one its model lists, `disabled`
	// for thinking off, or null for the model's default.
	ThinkingEffort **string `json:"thinkingEffort,omitzero" check:"nullable,chars=1.."`
	// The conversation's service tier: one its model lists, or null for the
	// vendor's default.
	ServiceTierID **string `json:"serviceTierId,omitzero" check:"nullable,chars=1.."`
	// A switch of the conversation's execution target.
	Target *ConversationTarget `json:"target,omitzero"`
}

// A provider entry of the user's scope and one of its models.
//
//demi:wire
type ModelChoice struct {
	ProviderID ProviderID `json:"providerId" check:"func=Validate"`
	ModelID    string     `json:"modelId" check:"chars=1.."`
}

// A field of a conversation patch, as its result names it.
//
//demi:enum
//demi:export
type PatchField string

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
//
//demi:union tag=status
//demi:export
type FieldResult interface{ isFieldResult() }

//demi:variant applied open
type FieldResultApplied struct {
	Field PatchField `json:"field"`
}

func (FieldResultApplied) isFieldResult() {}

//demi:variant failed open
type FieldResultFailed struct {
	Field      PatchField `json:"field"`
	Code       ErrorCode  `json:"code"`
	Message    string     `json:"message"`
	HTTPStatus uint16     `json:"httpStatus"`
}

func (FieldResultFailed) isFieldResult() {}

// The answer of a patch: the conversation as it is now, and each field's
// result in the order they were applied, the archive first.
//
//demi:wire open
type ConversationUpdate struct {
	Conversation ConversationSummary `json:"conversation"`
	Results      []FieldResult       `json:"results"`
}

// `POST /conversations/batch`: up to 100 patches, each of one of the
// caller's conversations.
//
//demi:wire
type ConversationBatch struct {
	Items []BatchItem `json:"items" check:"items=1..100"`
}

//demi:wire
type BatchItem struct {
	ID    ConversationID    `json:"id" check:"func=Validate"`
	Patch ConversationPatch `json:"patch"`
}

// The answer of a batch: an outcome per item, in the order given.
//
//demi:wire open
type BatchAnswer struct {
	Results []BatchResult `json:"results"`
}

// One item's outcome: its patch's answer, or why the item was refused,
// such as a conversation the caller does not have.
//
//demi:union tag=status
//demi:export
type BatchResult interface{ isBatchResult() }

//demi:variant updated open
type BatchResultUpdated struct {
	ID           ConversationID      `json:"id" check:"func=Validate"`
	Conversation ConversationSummary `json:"conversation"`
	Results      []FieldResult       `json:"results"`
}

func (BatchResultUpdated) isBatchResult() {}

//demi:variant refused open
type BatchResultRefused struct {
	ID      ConversationID `json:"id" check:"func=Validate"`
	Code    ErrorCode      `json:"code"`
	Message string         `json:"message"`
}

func (BatchResultRefused) isBatchResult() {}

// `POST /conversations/:id/fork`: the new conversation's id, chosen by the
// browser, and the completed assistant text the history is kept through.
//
//demi:wire
type ForkRequest struct {
	ID      ConversationID `json:"id" check:"func=Validate"`
	BlockID core.BlockID   `json:"blockId" check:"func=core.Validate"`
}

// The answer of a Fork: the new conversation, whose model settings are the
// ones it inherited from its source.
//
//demi:wire open
type ForkAnswer struct {
	Conversation ConversationSummary `json:"conversation"`
}
