package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// CreateConversation returns the owner's conversation of `id`, which is created when no
// conversation has the id: on the Cloud, with the placeholder title,
// first in the owner's sidebar. A retry of the owner's finds the one it
// created, in the spelling it was created with.
func (c *ControlService) CreateConversation(ctx context.Context, owner webapi.UserID, id webapi.ConversationID) (Creation, error) {
	panic("not written: b-database")
}

// Conversation returns the conversation of `id`, in whichever case it is spelled.
func (c *ControlService) Conversation(ctx context.Context, id webapi.ConversationID) (*ConversationRecord, error) {
	panic("not written: b-database")
}

// LastSwitch returns the conversation's latest target switch, which every node's next
// context block describes; none before its first.
func (c *ControlService) LastSwitch(ctx context.Context, id webapi.ConversationID) (*TargetSwitch, error) {
	panic("not written: b-database")
}

// Conversations returns the owner's conversations that are archived, or that are not, in
// sidebar order.
func (c *ControlService) Conversations(ctx context.Context, owner webapi.UserID, archived bool) ([]ConversationRecord, error) {
	panic("not written: b-database")
}

// ConversationOrder returns the ids of the owner's conversations in the order the product state
// lists them: the active ones in sidebar order, then the archived ones.
func (c *ControlService) ConversationOrder(ctx context.Context, owner webapi.UserID) ([]webapi.ConversationID, error) {
	panic("not written: b-database")
}

// MarkConversationRead acknowledges the output up to `revision`; an acknowledgement never
// moves the read revision back.
func (c *ControlService) MarkConversationRead(ctx context.Context, id webapi.ConversationID, revision uint64) error {
	panic("not written: b-database")
}

// ChangeConversation applies `change` in one transaction. An archived conversation takes
// nothing but its restore; a rename that repeats the current title
// changes nothing, so the title keeps its origin.
func (c *ControlService) ChangeConversation(ctx context.Context, id webapi.ConversationID, change RecordChange) (ChangeOutcome, error) {
	panic("not written: b-database")
}

// CountUserMessage counts one more message the user sent, which makes a generated title
// older than the conversation; answers how many there are now.
func (c *ControlService) CountUserMessage(ctx context.Context, id webapi.ConversationID) (uint64, error) {
	panic("not written: b-database")
}

// TitleFromFirstMessage makes `title`, from the first message, the conversation's while its
// title is still the placeholder; answers whether it did, which makes
// this send the one a generated title may follow.
func (c *ControlService) TitleFromFirstMessage(ctx context.Context, id webapi.ConversationID, title string) (bool, error) {
	panic("not written: b-database")
}

// GeneratedTitle writes the generated `title` while the title is still `from`, the one
// its request began from, in the statement that checks it, so a rename
// that landed meanwhile stays. Either way the title is current for the
// `seen` messages the request read. Answers whether it was written.
func (c *ControlService) GeneratedTitle(ctx context.Context, id webapi.ConversationID, title string, from string, seen uint64) (bool, error) {
	panic("not written: b-database")
}

// MarkLive records that the conversation's agent tree was live at `at`
// (`storage.md` § Retiring tool media). A record never moves back, so
// one written late cannot hide a later one.
func (c *ControlService) MarkLive(ctx context.Context, id webapi.ConversationID, at core.Timestamp) error {
	panic("not written: b-database")
}

// LiveAt returns when the conversation's agent tree was last seen live; none when there
// is no such conversation.
func (c *ControlService) LiveAt(ctx context.Context, id webapi.ConversationID) (*core.Timestamp, error) {
	panic("not written: b-database")
}

// SetWakeup records when the earliest wakeup the conversation's tree saved is due,
// or that it saved none (`runtime.md` § Yield wakeups).
func (c *ControlService) SetWakeup(ctx context.Context, id webapi.ConversationID, wakeup WakeupDue) error {
	panic("not written: b-database")
}

// SavedWakeups returns the conversations that are not archived and whose tree saved a
// wakeup, each with its owner and when its earliest wakeup is due,
// earliest first.
func (c *ControlService) SavedWakeups(ctx context.Context) ([]SavedWakeup, error) {
	panic("not written: b-database")
}

// TouchConversation records activity in the conversation now. Activity never reorders the
// sidebar.
func (c *ControlService) TouchConversation(ctx context.Context, id webapi.ConversationID) error {
	panic("not written: b-database")
}

// AttachedHosts returns the conversation's attached hosts, first attached first.
func (c *ControlService) AttachedHosts(ctx context.Context, id webapi.ConversationID) ([]AttachedHostRecord, error) {
	panic("not written: b-database")
}

// AttachedHostListing returns the conversation's attached hosts with when each was attached, first
// attached first, as the web app lists them.
func (c *ControlService) AttachedHostListing(ctx context.Context, id webapi.ConversationID) ([]AttachedHostListing, error) {
	panic("not written: b-database")
}

// SwitchConversationTarget returns the target switch's write, against the target the switch started
// from: false, writing nothing, when the target is no longer
// `expected`, so of two switches from one target exactly one wins. The
// winner records `switch` for every node's next context block and
// advances the execution-context revision.
func (c *ControlService) SwitchConversationTarget(ctx context.Context, id webapi.ConversationID, expected webapi.ConversationTarget, to webapi.ConversationTarget, switchValue TargetSwitch, ends SwitchEnds) (bool, error) {
	panic("not written: b-database")
}

// SetAttachedCWD records where the last `demi host shell --host` on the attached
// `device` ended, which is where the next one there starts.
func (c *ControlService) SetAttachedCWD(ctx context.Context, id webapi.ConversationID, device webapi.DeviceID, cwd string) error {
	panic("not written: b-database")
}
