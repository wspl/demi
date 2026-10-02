package hostaccess

//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// ConversationBlobs adapts the owner's namespace and use records for storage.
type ConversationBlobs struct{ Namespace *blobs.Namespace }

// Media is the namespace as the conversation's sessions reach it.
func (b ConversationBlobs) Media() store.BlobStore { panic("not written: b-hostaccess") }

// CommitUses records references inside a database commit without network IO.
func (b ConversationBlobs) CommitUses(ctx context.Context, refs []core.BlobRef) error {
	panic("not written: b-hostaccess")
}

// ConversationHostForNode resolves the current main Host for an agent node.
// The returned handle is for shell environment identity; its jobs must use
// RunJob, and other file operations must use WithHost for their whole lifetime.
func ConversationHostForNode(ctx context.Context, shard HostShard, id webapi.ConversationID) (*remotehost.Host, error) {
	panic("not written: b-hostaccess")
}

// RunJob admits one shell job, checks its Host identity, installs the user's
// plugin directories once per connection and revision, then runs it once.
// JobEnded runs on every completion, failure or cancellation after dispatch.
func RunJob(ctx context.Context, shard HostShard, id webapi.ConversationID, key host.Key, job func(context.Context) error) error {
	panic("not written: b-hostaccess")
}

// ShardShellEnvironments makes node-owned shell environments and keeps outputs
// and edit copies as the user's blobs. The shard closes environments before its
// dependencies; the factory owns no goroutines independently of those scopes.
type ShardShellEnvironments struct{}

// NewShardShellEnvironments binds a shard and its published command catalog.
func NewShardShellEnvironments(shard HostShard, catalog *remotehost.CommandCatalog) *ShardShellEnvironments {
	panic("not written: b-hostaccess")
}

// Create registers the node's commands and makes its environment on target.
// The receiver owns and closes the returned environment.
func (s *ShardShellEnvironments) Create(ctx context.Context, scope tools.EnvironmentScope, target *remotehost.Host) (host.ShellEnvironment, error) {
	panic("not written: b-hostaccess")
}

var _ database.OwnerBlobs = ConversationBlobs{}
var _ tools.ShellEnvironmentFactory[*remotehost.Host] = (*ShardShellEnvironments)(nil)
