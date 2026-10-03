package cloud

import (
	"context"
	"errors"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/webapi"
)

// ErrNotLetGo means a conversation did not let go of its tree or file gate within the hold time.
var ErrNotLetGo = errors.New("the conversation did not let go in time")

// CloudShard supplies what the Cloud needs of its user's shard: the handles
// its operations use, and the user's conversations, which keep the Cloud awake
// while they work and which an idle stop and a reset hold.
// Handles and configuration remain stable for the shard's lifetime. Callbacks
// synchronize their own access to shard state and are called without the shard
// mutex held. Cloud owns and joins the workers that use this interface; the
// shard must call Close before disposing any of these handles.
//
//nolint:revive // The architecture names this cross-package boundary CloudShard.
type CloudShard interface {
	// User identifies the owner.
	User() webapi.UserID
	// Cloud is the user's Cloud machine.
	Cloud() *Cloud
	// Devices is the user's devices, the Cloud among them.
	Devices() *runners.Devices
	// Control supplies the user's durable control records.
	Control() *database.ControlService
	// CloudServices supplies the shared manager client, capacity and settings.
	CloudServices() *Services
	// PublicURL is where a booting Cloud's runner reaches this backend.
	PublicURL() *runners.PublicURL
	// IdleWindow is how long a Cloud no conversation uses stays awake.
	IdleWindow() time.Duration
	// Marks is the user's pages, which show the Cloud and the devices.
	Marks() pagesync.UserMarks
	// Vault is the credential vault, whose entries say which providers run a
	// process on the Cloud.
	Vault() *providers.Vault
	// Assembly resolves which providers need a process.
	Assembly() *providers.Assembly
	// Activity is what the conversation is doing: a turn of its tree, an
	// operation holding its file gate, or a user stream someone has open.
	Activity(conversation webapi.ConversationID) idlewatch.Activity
	// Attended reports whether someone attends the conversation: a turn of it
	// is in flight, or a file transfer or user stream of it is open.
	Attended(conversation webapi.ConversationID) bool
	// HoldForIdle holds the conversation's tree and file gate now if neither
	// works. Nil means either is held; a failed attempt releases partial holds.
	HoldForIdle(conversation webapi.ConversationID) ConversationHold
	// HoldForReset interrupts the turn and holds its tree. If filesOnCloud,
	// it ends file transfers and user streams, then reserves the file gate
	// once its operations end. Each wait has hold; if one runs out, it returns
	// ErrNotLetGo. Cancellation returns ctx.Err().
	// Failure releases partial holds; success transfers Release to the caller.
	HoldForReset(
		ctx context.Context,
		conversation webapi.ConversationID,
		filesOnCloud bool,
		hold time.Duration,
	) (ConversationHold, error)
	// CloudStopped ends the exposes of a Cloud that stops or stopped.
	CloudStopped(ctx context.Context, device webapi.DeviceID) error
}

// ConversationHold is what the shard holds of one conversation for an idle
// stop or a reset of the Cloud. Its owner defers Release.
type ConversationHold interface {
	// Release lets the conversation go. It is idempotent and does not wait.
	Release()
}
