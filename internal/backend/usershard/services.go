package usershard

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"net/url"
	"path/filepath"

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
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/provider"
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
	control, err := database.OpenControl(ctx, filepath.Join(dataDir, "control.sqlite"), clock)
	if err != nil {
		return nil, err
	}
	conversations, err := database.OpenConversations(ctx, filepath.Join(dataDir, "conversations"), database.MaxWriters)
	if err != nil {
		closeErr := control.Close(context.WithoutCancel(ctx))
		if closeErr != nil {
			slog.ErrorContext(ctx, "the control database did not close after a failed start", "error", closeErr)
		}
		return nil, errors.Join(err, closeErr)
	}
	return &Storage{Control: control, Conversations: conversations, Blobs: blobs.New(objects, clock)}, nil
}

// Close closes conversation databases before control, returning all failures.
func (s *Storage) Close(ctx context.Context) error {
	var failures []error
	if err := s.Conversations.Close(ctx); err != nil {
		failures = append(failures, &CloseError{Kind: CloseConversation, Err: err})
	}
	if err := s.Control.Close(ctx); err != nil {
		failures = append(failures, &CloseError{Kind: CloseControl, Err: err})
	}
	return errors.Join(failures...)
}

// StartServices starts the shared services. The caller retains Storage and
// closes services before closing storage; ctx owns their background work.
func StartServices(ctx context.Context, storage *Storage, keys ServiceKeys, setup ProviderSetup, settings ServiceSettings) (*Services, error) {
	registry, err := plugins.NewRegistry(settings.Plugins, func(op declare.NativeOperation) bool {
		return settings.Native.Serves(op.Package, []string{op.Operation})
	})
	if err != nil {
		return nil, &ServicesError{Kind: ServicesPlugins, Err: err}
	}
	hasher, err := accounts.NewPasswordHasher(ctx)
	if err != nil {
		return nil, &ServicesError{Kind: ServicesHashing, Err: err}
	}
	releases, err := providers.NewClaudeReleases(setup.ClaudeReleases)
	if err != nil {
		return nil, &ServicesError{Kind: ServicesHTTP, Err: err}
	}
	syncs := &pagesync.SyncRegistry{}
	vault := providers.NewVault(storage.Control, &keys.Vault, settings.Mode, syncs)
	httpClient := &http.Client{}
	models := provider.NewModelsDevClient(httpClient, setup.ModelsDevURL.String(), setup.Clock)
	assembly := providers.NewAssembly(vault, setup.Families, providers.NewAccountQuotas(vault), providers.NewModelCatalogCache(storage.Control, setup.Clock), providers.NewVendorCatalog(models), httpClient, setup.Clock)
	operations := &providers.Operations{}
	var streams []hostaccess.StreamDeclaration
	for _, stream := range registry.Streams() {
		streams = append(streams, hostaccess.StreamDeclaration{Name: string(stream.Name), Operation: stream.Operation})
	}
	return &Services{
		Mode: settings.Mode, Clock: setup.Clock, Control: storage.Control, Conversations: storage.Conversations, Blobs: storage.Blobs,
		Hasher: hasher, Sessions: accounts.NewWebSessions(storage.Control), Limiter: accounts.NewLoginLimiter(), Email: accounts.NewEmailChanges(storage.Control, hasher, settings.Mail, keys.EmailCodes),
		Vault: vault, Assembly: assembly, ClaudeReleases: releases, CLIInstalls: &CLIInstalls{}, Operations: operations, Logins: providers.NewLoginFlows(assembly, operations, setup.Logins),
		Claims: runners.NewPendingClaims(settings.Runners.ClaimsPerMinute), Runners: settings.Runners, ConversationTuning: settings.Conversations, Pages: settings.Pages, Native: settings.Native, Plugins: registry,
		UserStreams: hostaccess.NewUserStreams(streams, settings.Native), Cloud: settings.Cloud, PublicURL: &runners.PublicURL{}, Lifecycle: settings.Lifecycle, ExposeDomain: settings.ExposeDomain, ExposeTuning: settings.Exposes, Sync: syncs,
	}, nil
}

// CloseProviders cancels logins and joins catalog refreshes and quota writes.
func (s *Services) CloseProviders(ctx context.Context) error {
	loginErr := s.Logins.Close(ctx)
	assemblyErr := s.Assembly.Close(ctx)
	releaseErr := s.ClaudeReleases.Close(ctx)
	return errors.Join(loginErr, assemblyErr, releaseErr)
}

// Close joins shared service work after all shards close and before storage closes.
func (s *Services) Close(ctx context.Context) error {
	s.Claims.Close()
	managerErr := s.Cloud.Machines.Close(ctx)
	providerErr := s.CloseProviders(ctx)
	return errors.Join(managerErr, providerErr)
}

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
