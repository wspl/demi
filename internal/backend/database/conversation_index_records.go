package database

import (
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// ConversationRecord is a conversation as the index holds it.
type ConversationRecord struct {
	ID       webapiproto.ConversationID
	Owner    webapiproto.UserID
	Title    string
	Archived bool
	Pinned   bool
	// The output revision the user last acknowledged; it only moves
	// forward.
	ReadRevision uint64
	Target       webapiproto.ConversationTarget
	// Advanced by every change of the conversation's execution context,
	// such as a target switch or a Host attached.
	ContextVersion uint64
	// The conversation's model selection; none until its first model is
	// chosen.
	Model *types.ModelSelection
	// How many messages the user has sent, and how many of them the title
	// has read (`product.md` § Conversation titles).
	UserMessages   uint64
	TitledMessages uint64
	CreatedAt      types.Timestamp
	UpdatedAt      types.Timestamp
	// The revision of the conversation's draft, 0 before its first save.
	DraftRevision uint64
	// PanelRevision counts changes to the conversation panel.
	PanelRevision uint64
}

// SavedWakeup is a conversation whose tree saved a yield wakeup, with its owner and when
// its earliest wakeup is due.
type SavedWakeup struct {
	Conversation webapiproto.ConversationID
	Owner        webapiproto.UserID
	Due          WakeupDue
}

// SettingsChange is a change of a conversation's model settings: the parts a patch names.
type SettingsChange struct {
	// A switch to this model, with the effort and the tier below and the
	// model's defaults for a part they leave out.
	Model *webapiproto.ModelChoice
	// The thinking effort, or null for the model's default.
	ThinkingEffort **string
	// The service tier, or null for the vendor's default.
	ServiceTierID **string
}

// TargetColumns is a target as its typed columns: the kind and what the kind names.
type TargetColumns struct {
	Kind      string
	Device    *string
	Path      *string
	Workspace *string
}

// NewConversation is a conversation as it enters the index, seen live as it is created.
type NewConversation struct {
	ID     webapiproto.ConversationID
	Owner  webapiproto.UserID
	Title  string
	Origin TitleOrigin
	Target webapiproto.ConversationTarget
	Model  *types.ModelSelection
	// When it was created, and last active.
	At types.Timestamp
}

// SwitchEnds describes the two ends of a target switch.
type SwitchEnds struct {
	// The device the switch leaves, attached afterwards where it was left.
	Departed *DepartedHost
	// The device the switch reaches, detached if it was attached: a Host is
	// main or attached, never both.
	Arriving *webapiproto.DeviceID
}
