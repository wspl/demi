package usershard

//revive:disable:unused-parameter

import (
	"context"
	"net/url"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// Services are shared by the edge and every user's shard. Their handles are
// immutable after startup; each service synchronizes its own mutable state.
type Services struct {
	Mode               webapi.InstanceMode
	Clock              core.Clock
	Control            *database.ControlService
	Conversations      *database.ConversationStores
	Blobs              *blobs.Stores
	Hasher             *accounts.PasswordHasher
	Sessions           *accounts.WebSessions
	Limiter            *accounts.LoginLimiter
	Email              *accounts.EmailChanges
	Vault              *providers.Vault
	Assembly           *providers.Assembly
	ClaudeReleases     *providers.ClaudeReleases
	CLIInstalls        *CLIInstalls
	Operations         *providers.Operations
	Logins             *providers.LoginFlows
	Claims             *runners.PendingClaims
	Runners            RunnerTuning
	ConversationTuning ConversationTuning
	Pages              PageTuning
	Native             *runners.NativeCatalog
	Plugins            *plugins.Registry
	UserStreams        *hostaccess.UserStreams
	Cloud              *cloud.Services
	PublicURL          *runners.PublicURL
	Lifecycle          LifecycleTuning
	ExposeDomain       *expose.Domain
	ExposeTuning       ExposeTuning
	Sync               *pagesync.SyncRegistry
	// Hooks are optional synchronization points supplied by test support.
	Hooks FlowHooks
}

// ProviderSetup supplies the provider families, catalogs, timing and clock.
type ProviderSetup struct {
	Families       *providers.FamilyRegistry
	ModelsDevURL   *url.URL
	ClaudeReleases *url.URL
	Logins         providers.LoginTiming
	Clock          core.Clock
}

// ServiceKeys are derived from the instance secret by the executable.
type ServiceKeys struct {
	Vault      providers.VaultKey
	EmailCodes accounts.CodeKey
}

// ServiceSettings configures services independently of storage and keys.
type ServiceSettings struct {
	Mode          webapi.InstanceMode
	Mail          accounts.AccountMail
	Runners       RunnerTuning
	Conversations ConversationTuning
	Pages         PageTuning
	Native        *runners.NativeCatalog
	Plugins       []plugin.Factory
	Cloud         *cloud.Services
	Lifecycle     LifecycleTuning
	ExposeDomain  *expose.Domain
	Exposes       ExposeTuning
}

// Storage holds the databases and object namespaces used by the services.
type Storage struct {
	Control       *database.ControlService
	Conversations *database.ConversationStores
	Blobs         *blobs.Stores
}

// OpenStorage opens the databases in dataDir beside objects. A failed open
// closes everything opened by that attempt.
func OpenStorage(ctx context.Context, dataDir string, clock core.Clock, objects blobs.Objects) (*Storage, error) {
	panic("not written: b-usershard")
}

// Close closes conversation databases before control, returning all failures.
func (s *Storage) Close(ctx context.Context) error { panic("not written: b-usershard") }

// StartServices starts the shared services. The caller retains Storage and
// closes services before closing storage; ctx owns their background work.
func StartServices(ctx context.Context, storage *Storage, keys ServiceKeys, setup ProviderSetup, settings ServiceSettings) (*Services, error) {
	panic("not written: b-usershard")
}

// CloseProviders cancels logins and joins catalog refreshes and quota writes.
func (s *Services) CloseProviders(ctx context.Context) error { panic("not written: b-usershard") }

// Close joins shared service work after all shards close and before storage closes.
func (s *Services) Close(ctx context.Context) error { panic("not written: b-usershard") }

// FlowHooks lets test support hold a flow without importing test code here.
// Hooks run without the shard mutex held and honor cancellation.
type FlowHooks interface {
	// Hello waits at a runner handshake step.
	Hello(ctx context.Context, step HelloStep) error
	// Sync waits at a page synchronization step.
	Sync(ctx context.Context, step SyncStep) error
}

// HelloStep identifies a runner hello's synchronization point.
type HelloStep uint8

const (
	// HelloTokenLookup is the edge's token lookup while it watches socket closure.
	HelloTokenLookup HelloStep = iota
	// HelloBind precedes binding the accepted socket in its owner's shard.
	HelloBind
)

// SyncStep identifies a page channel's synchronization point.
type SyncStep uint8

const (
	// SyncSnapshot follows reading initial product state and precedes sending it.
	SyncSnapshot SyncStep = iota
	// SyncChanges follows a change wakeup and precedes reading changed parts.
	SyncChanges
)
