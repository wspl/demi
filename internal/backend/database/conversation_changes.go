package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ConversationChange is a user-requested change applied by the shard.
//
//sumtype:decl
type ConversationChange interface{ conversationChange() }

// ConversationRecordChange changes the conversation record.
type ConversationRecordChange struct{ Change RecordChange }

func (*ConversationRecordChange) conversationChange() {}

// ConversationSettingsChange changes model settings against the catalog.
type ConversationSettingsChange struct{ Change SettingsChange }

func (*ConversationSettingsChange) conversationChange() {}

// ConversationTargetChange switches the main target by compare-and-set.
type ConversationTargetChange struct{ Target webapi.ConversationTarget }

func (*ConversationTargetChange) conversationChange() {}

// RecordChange is a record change applied by one index transaction.
//
//sumtype:decl
type RecordChange interface{ recordChange() }

// RecordArchived archives or restores.
type RecordArchived struct{ Archived bool }

func (*RecordArchived) recordChange() {}

// RecordTitle renames to a user-owned title.
type RecordTitle struct{ Title string }

func (*RecordTitle) recordChange() {}

// RecordPinned changes pin state.
type RecordPinned struct{ Pinned bool }

func (*RecordPinned) recordChange() {}

// RecordModel changes the model selection.
type RecordModel struct{ Model core.ModelSelection }

func (*RecordModel) recordChange() {}

// RecordAttach attaches a device, leaving an existing attachment unchanged.
type RecordAttach struct{ Host AttachedHostRecord }

func (*RecordAttach) recordChange() {}

// RecordRename renames an attached host uniquely within the conversation.
type RecordRename struct {
	Device webapi.DeviceID
	Name   string
}

func (*RecordRename) recordChange() {}

// RecordDetach detaches a device, doing nothing when it was not attached.
type RecordDetach struct{ Device webapi.DeviceID }

func (*RecordDetach) recordChange() {}

// Creation describes what creating a conversation ID found.
//
//sumtype:decl
type Creation interface{ creation() }

// ConversationCreated holds the new indexed conversation.
type ConversationCreated struct{ Record ConversationRecord }

func (*ConversationCreated) creation() {}

// ConversationExisting holds the owner's existing conversation found by a retry.
type ConversationExisting struct{ Record ConversationRecord }

func (*ConversationExisting) creation() {}

// ConversationUnavailable means another owner or a Fork holds the ID.
type ConversationUnavailable struct{}

func (*ConversationUnavailable) creation() {}

// ChangeOutcome describes what a record change found.
type ChangeOutcome uint8

const (
	// ChangeApplied means the change was applied.
	ChangeApplied ChangeOutcome = iota
	// ChangeMissing means no conversation has the ID.
	ChangeMissing
	// ChangeArchived means an archived conversation refused a change other than restore.
	ChangeArchived
	// ChangeNotAttached means the renamed device is not attached.
	ChangeNotAttached
	// ChangeNameTaken means another attached host has the name.
	ChangeNameTaken
)

// TitleOrigin records where a title came from.
type TitleOrigin uint8

const (
	// TitlePlaceholder is the initial generated placeholder.
	TitlePlaceholder TitleOrigin = iota
	// TitleUser is a user's title.
	TitleUser
)

// AttachedHostListing is an attached host and when it was attached.
type AttachedHostListing struct {
	Host AttachedHostRecord
	At   core.Timestamp
}

// DepartedHost is a device left by a target switch and its directory.
type DepartedHost struct {
	Device webapi.DeviceID
	Path   string
}

// ColumnsForTarget returns a target as its typed SQL columns.
func ColumnsForTarget(target webapi.ConversationTarget) TargetColumns {
	panic("not written: b-database")
}
