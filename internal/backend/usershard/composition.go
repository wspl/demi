package usershard

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"encoding/json"
	"time"

	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// User identifies the owner.
func (s *Shard) User() webapi.UserID { panic("not written: b-usershard") }

// Cloud is the user's Cloud machine.
func (s *Shard) Cloud() *cloud.Cloud { panic("not written: b-usershard") }

// Devices is the user's devices, the Cloud among them.
func (s *Shard) Devices() *runners.Devices { panic("not written: b-usershard") }

// Control supplies the user's durable control records.
func (s *Shard) Control() *database.ControlService { panic("not written: b-usershard") }

// CloudServices supplies the shared manager client, capacity and settings.
func (s *Shard) CloudServices() *cloud.Services { panic("not written: b-usershard") }

// PublicURL is where a booting Cloud's runner reaches this backend.
func (s *Shard) PublicURL() *runners.PublicURL { panic("not written: b-usershard") }

// IdleWindow is how long a Cloud no conversation uses stays awake.
func (s *Shard) IdleWindow() time.Duration { panic("not written: b-usershard") }

// Marks is the user's pages, which show the Cloud and the devices.
func (s *Shard) Marks() pagesync.UserMarks { panic("not written: b-usershard") }

// Vault is the credential vault, whose entries say which providers run a
// process on the Cloud.
func (s *Shard) Vault() *providers.Vault { panic("not written: b-usershard") }

// Assembly resolves which providers need a process.
func (s *Shard) Assembly() *providers.Assembly { panic("not written: b-usershard") }

// Activity is what the conversation is doing: a turn of its tree, an
// operation holding its file gate, or a user stream someone has open.
func (s *Shard) Activity(conversation webapi.ConversationID) idlewatch.Activity {
	panic("not written: b-usershard")
}

// Attended reports whether someone attends the conversation: a turn of it
// is in flight, or a file transfer or user stream of it is open.
func (s *Shard) Attended(conversation webapi.ConversationID) bool { panic("not written: b-usershard") }

// HoldForIdle holds the conversation's tree and file gate now if neither
// works. Nil means either is held; a failed attempt releases partial holds.
func (s *Shard) HoldForIdle(conversation webapi.ConversationID) cloud.ConversationHold {
	panic("not written: b-usershard")
}

// HoldForReset interrupts the turn and holds its tree. If filesOnCloud,
// it ends file transfers and user streams, then reserves the file gate
// once its operations end. Each wait has hold; nil, nil means the
// conversation did not let go in time. Cancellation returns ctx.Err().
// Failure releases partial holds; success transfers Release to the caller.
func (s *Shard) HoldForReset(ctx context.Context, conversation webapi.ConversationID, filesOnCloud bool, hold time.Duration) (cloud.ConversationHold, error) {
	panic("not written: b-usershard")
}

// CloudStopped ends the exposes of a Cloud that stops or stopped.
func (s *Shard) CloudStopped(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-usershard")
}

// Clock is the wall clock the backend reads times from.
func (s *Shard) Clock() core.Clock { panic("not written: b-usershard") }

// Pipes is the pipes of the user's devices.
func (s *Shard) Pipes() *remotehost.Pipes { panic("not written: b-usershard") }

// Commands is each agent node's commands, for the rpc calls of its jobs.
func (s *Shard) Commands() *runners.CommandRouter { panic("not written: b-usershard") }

// Conversations owns each conversation's slot, file gate and transfers.
func (s *Shard) Conversations() *hostaccess.Conversations { panic("not written: b-usershard") }

// Blobs is the user's blob namespace.
func (s *Shard) Blobs() *blobs.Namespace { panic("not written: b-usershard") }

// ConversationDB is the database of the user's conversation.
func (s *Shard) ConversationDB(conversation webapi.ConversationID) *database.ConversationDB {
	panic("not written: b-usershard")
}

// Native is the command packages the conversations' commands bind to.
func (s *Shard) Native() *runners.NativeCatalog { panic("not written: b-usershard") }

// CloudShard is the shard as the user's Cloud sees it.
func (s *Shard) CloudShard() cloud.CloudShard { panic("not written: b-usershard") }

// TrackIdle starts the conversation's idle watch unless one runs.
func (s *Shard) TrackIdle(conversation webapi.ConversationID) { panic("not written: b-usershard") }

// DirectorySets is the Host directories of the user's plugins.
func (s *Shard) DirectorySets(ctx context.Context) (hostaccess.DirectorySets, error) {
	panic("not written: b-usershard")
}

// PluginInstalls remembers the Hosts' installed directories.
func (s *Shard) PluginInstalls() *hostaccess.PluginInstalls { panic("not written: b-usershard") }

// JobEnded reports a finished or stopped job, which may change plugin views.
func (s *Shard) JobEnded(conversation webapi.ConversationID) { panic("not written: b-usershard") }

// PackageCall runs operation on the conversation's main Host, waking it as kind
// specifies. Args is a JSON object preserving its input member order.
func (s *Shard) PackageCall(ctx context.Context, conversation webapi.ConversationID, operation declare.NativeOperation, args json.RawMessage, kind plugin.CallKind) (json.RawMessage, error) {
	panic("not written: b-usershard")
}

// ConversationHosts lists the conversation's main and attached Hosts.
func (s *Shard) ConversationHosts(ctx context.Context, conversation webapi.ConversationID) ([]plugin.ConversationHost, error) {
	panic("not written: b-usershard")
}

// ReadHostFiles never wakes a Host; a stopped Host returns plugin.PortRefusalNotRunning.
func (s *Shard) ReadHostFiles(ctx context.Context, conversation webapi.ConversationID, reads []plugin.HostRead) ([]plugin.HostFile, error) {
	panic("not written: b-usershard")
}

// PutBlob stores bytes in the user's blob namespace.
func (s *Shard) PutBlob(ctx context.Context, bytes core.B64Bytes) (core.BlobRef, error) {
	panic("not written: b-usershard")
}

// Blob returns the user's blob bytes, or nil when absent.
func (s *Shard) Blob(ctx context.Context, blob core.BlobRef) (*core.B64Bytes, error) {
	panic("not written: b-usershard")
}

// BlobUses returns the record updated before a plugin value or directory write commits.
func (s *Shard) BlobUses() database.OwnerBlobs { panic("not written: b-usershard") }

// Exposes lists the user's live exposes, soonest expiry first.
func (s *Shard) Exposes(ctx context.Context) (plugin.ExposeList, error) {
	panic("not written: b-usershard")
}

// CreateExpose exposes address on device for lifetime seconds.
func (s *Shard) CreateExpose(ctx context.Context, device webapi.DeviceID, address string, lifetime uint64) (plugin.ExposeRecord, error) {
	panic("not written: b-usershard")
}

// RenewExpose moves expiry to lifetime seconds from now.
func (s *Shard) RenewExpose(ctx context.Context, expose webapi.ExposeID, lifetime uint64) (plugin.ExposeRecord, error) {
	panic("not written: b-usershard")
}

// RemoveExpose destroys the expose at once.
func (s *Shard) RemoveExpose(ctx context.Context, expose webapi.ExposeID) error {
	panic("not written: b-usershard")
}

var _ cloud.CloudShard = (*Shard)(nil)
var _ hostaccess.HostShard = (*Shard)(nil)
var _ plugins.PluginShard = (*Shard)(nil)
