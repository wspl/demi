package plugins

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// PluginShard supplies the services the plugin host needs of its user's shard.
// Host operations go through the conversation's host access. Implementations
// support concurrent calls, including ports retained by plugin-owned work.
// Operation errors preserve plugin.PortRefusal or host.PortError for errors.As.
type PluginShard interface {
	// Control returns the control service holding the user's plugin values and choices.
	Control() *database.ControlService
	// Marks returns the user's page-state change marks.
	Marks() pagesync.UserMarks
	// PackageCall runs operation on the conversation's main Host, waking it as kind
	// specifies. Args is a JSON object preserving its input member order.
	PackageCall(ctx context.Context, conversation webapi.ConversationID, operation declare.NativeOperation, args json.RawMessage, kind plugin.CallKind) (json.RawMessage, error)
	// ConversationHosts lists the conversation's main and attached Hosts.
	ConversationHosts(ctx context.Context, conversation webapi.ConversationID) ([]plugin.ConversationHost, error)
	// ReadHostFiles never wakes a Host; a stopped Host returns plugin.PortRefusalNotRunning.
	ReadHostFiles(ctx context.Context, conversation webapi.ConversationID, reads []plugin.HostRead) ([]plugin.HostFile, error)
	// PutBlob stores bytes in the user's blob namespace.
	PutBlob(ctx context.Context, bytes core.B64Bytes) (core.BlobRef, error)
	// Blob returns the user's blob bytes, or nil when absent.
	Blob(ctx context.Context, blob core.BlobRef) (*core.B64Bytes, error)
	// BlobUses returns the record updated before a plugin value or directory write commits.
	BlobUses() database.OwnerBlobs
	// Exposes lists the user's live exposes, soonest expiry first.
	Exposes(ctx context.Context) (plugin.ExposeList, error)
	// CreateExpose exposes address on device for lifetime seconds.
	CreateExpose(ctx context.Context, device webapi.DeviceID, address string, lifetime uint64) (plugin.ExposeRecord, error)
	// RenewExpose moves expiry to lifetime seconds from now.
	RenewExpose(ctx context.Context, expose webapi.ExposeID, lifetime uint64) (plugin.ExposeRecord, error)
	// RemoveExpose destroys the expose at once.
	RemoveExpose(ctx context.Context, expose webapi.ExposeID) error
}
