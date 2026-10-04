package database

import (
	"errors"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
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
type ConversationTargetChange struct {
	Target webapiproto.ConversationTarget
}

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
type RecordModel struct{ Model types.ModelSelection }

func (*RecordModel) recordChange() {}

// RecordAttach attaches a device, leaving an existing attachment unchanged.
type RecordAttach struct{ Host AttachedHostRecord }

func (*RecordAttach) recordChange() {}

// RecordRename renames an attached host uniquely within the conversation.
type RecordRename struct {
	Device webapiproto.DeviceID
	Name   string
}

func (*RecordRename) recordChange() {}

// RecordDetach detaches a device, doing nothing when it was not attached.
type RecordDetach struct{ Device webapiproto.DeviceID }

func (*RecordDetach) recordChange() {}

// ErrIDUnavailable means another owner or a Fork holds the conversation ID.
var ErrIDUnavailable = errors.New("conversation id is unavailable")

var (
	// ErrConversationNotFound means no conversation has the ID.
	ErrConversationNotFound = errors.New("no such conversation")
	// ErrArchived means the conversation is archived and takes nothing but its restore.
	ErrArchived = errors.New("conversation is archived")
	// ErrNotAttached means the renamed device is not attached.
	ErrNotAttached = errors.New("the device is not attached")
	// ErrNameTaken means another attached host has the name.
	ErrNameTaken = errors.New("another attached host has the name")
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
	At   types.Timestamp
}

// DepartedHost is a device left by a target switch and its directory.
type DepartedHost struct {
	Device webapiproto.DeviceID
	Path   string
}

// ColumnsForTarget returns a target as its typed SQL columns.
func ColumnsForTarget(target webapiproto.ConversationTarget) TargetColumns {
	switch target := target.(type) {
	case *webapiproto.ConversationTargetCloud:
		return TargetColumns{Kind: "cloud", Path: target.Path}
	case *webapiproto.ConversationTargetDevice:
		device := string(target.DeviceID)
		return TargetColumns{Kind: "device", Device: &device, Path: &target.Path}
	case *webapiproto.ConversationTargetWorkspace:
		workspace := string(target.WorkspaceID)
		return TargetColumns{Kind: "workspace", Workspace: &workspace}
	}
	return TargetColumns{}
}
