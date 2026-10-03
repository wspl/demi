package usershard

import (
	"context"
	"errors"
	"fmt"
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
	// Mode selects the instance’s user isolation policy.
	Mode webapi.InstanceMode
	// Clock supplies timestamps for the shared services.
	Clock core.Clock
	// Control provides access to persisted control records.
	Control *database.ControlService
	// Conversations provides per-conversation database storage.
	Conversations *database.ConversationStores
	// Blobs provides the users’ object namespaces.
	Blobs *blobs.Stores
	// Hasher hashes and verifies account passwords.
	Hasher *accounts.PasswordHasher
	// Sessions authenticates browser sessions.
	Sessions *accounts.WebSessions
	// Limiter enforces account login admission.
	Limiter *accounts.LoginLimiter
	// Email owns pending account email changes.
	Email *accounts.EmailChanges
	// Vault provides sealed provider credential storage.
	Vault *providers.Vault
	// Assembly assembles provider instances from their configuration.
	Assembly *providers.Assembly
	// ClaudeReleases supplies Claude Code releases for Cloud installs.
	ClaudeReleases *providers.ClaudeReleases
	// CLIInstalls records the state of requested Cloud CLI installations.
	CLIInstalls *CLIInstalls
	// Operations serializes provider configuration operations.
	Operations *providers.Operations
	// Logins owns provider login flows and their results.
	Logins *providers.LoginFlows
	// Claims owns pending runner pairing claims.
	Claims *runners.PendingClaims
	// Runners configures runner connections and admission.
	Runners RunnerTuning
	// ConversationTuning configures conversation execution limits.
	ConversationTuning ConversationTuning
	// Pages configures browser page connections.
	Pages PageTuning
	// Native supplies the deployment’s native command packages.
	Native *runners.NativeCatalog
	// Plugins supplies registered plugins in their declared order.
	Plugins *plugins.Registry
	// UserStreams owns user-scoped host streams.
	UserStreams *hostaccess.UserStreams
	// Cloud provides Cloud lifecycle services.
	Cloud *cloud.Services
	// PublicURL supplies the backend’s public URL.
	PublicURL *runners.PublicURL
	// Lifecycle configures shard lifecycle and retention.
	Lifecycle LifecycleTuning
	// ExposeDomain supplies the domain for public exposes.
	ExposeDomain *expose.Domain
	// ExposeTuning configures expose admission and lifetimes.
	ExposeTuning ExposeTuning
	// Sync publishes page state change marks.
	Sync *pagesync.SyncRegistry
	// Hooks are optional synchronization points supplied by test support.
	Hooks FlowHooks
}

// ProviderSetup supplies the provider families, catalogs, timing and clock.
type ProviderSetup struct {
	// Families registers the provider families available in the deployment.
	Families *providers.FamilyRegistry
	// ModelsDevURL locates the vendor and model catalog service.
	ModelsDevURL *url.URL
	// ClaudeReleases locates the Claude Code release service.
	ClaudeReleases *url.URL
	// Logins configures login lifetime and result retention.
	Logins providers.LoginTiming
	// Clock supplies timestamps for the shared services.
	Clock core.Clock
}

// ServiceKeys are derived from the instance secret by the executable.
type ServiceKeys struct {
	// Vault protects persisted provider secrets.
	Vault providers.VaultKey
	// EmailCodes protects account email verification codes.
	EmailCodes accounts.CodeKey
}

// ServiceSettings configures services independently of storage and keys.
type ServiceSettings struct {
	// Mode selects the instance’s user isolation policy.
	Mode webapi.InstanceMode
	// Mail delivers account email messages.
	Mail accounts.AccountMail
	// Runners configures runner connections and admission.
	Runners RunnerTuning
	// Conversations configures conversation execution limits.
	Conversations ConversationTuning
	// Pages configures browser page connections.
	Pages PageTuning
	// Native supplies the deployment’s native command packages.
	Native *runners.NativeCatalog
	// Plugins lists plugin factories in registration order.
	Plugins []plugin.Factory
	// Cloud provides Cloud lifecycle services.
	Cloud *cloud.Services
	// Lifecycle configures shard lifecycle and retention.
	Lifecycle LifecycleTuning
	// ExposeDomain supplies the domain for public exposes.
	ExposeDomain *expose.Domain
	// Exposes configures expose admission and lifetimes.
	Exposes ExposeTuning
}

// Storage holds the databases and object namespaces used by the services.
type Storage struct {
	// Control provides access to persisted control records.
	Control *database.ControlService
	// Conversations provides per-conversation database storage.
	Conversations *database.ConversationStores
	// Blobs provides the users’ object namespaces.
	Blobs *blobs.Stores
}

// OpenStorage opens the databases in dataDir beside objects. A failed open
// closes everything opened by that attempt.
func OpenStorage(
	ctx context.Context,
	dataDir string,
	clock core.Clock,
	objects blobs.Objects,
) (*Storage, error) {
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
	return &Storage{
		Control:       control,
		Conversations: conversations,
		Blobs:         blobs.New(objects, clock),
	}, nil
}

// Close closes conversation databases before control, returning all failures.
func (s *Storage) Close(ctx context.Context) error {
	var failures []error
	if err := s.Conversations.Close(ctx); err != nil {
		failures = append(failures, fmt.Errorf("a conversation database did not close: %w", err))
	}
	if err := s.Control.Close(ctx); err != nil {
		failures = append(failures, fmt.Errorf("the control database did not close: %w", err))
	}
	return errors.Join(failures...)
}

// StartServices starts the shared services. The caller retains Storage and
// closes services before closing storage; ctx owns their background work.
func StartServices(
	ctx context.Context,
	storage *Storage,
	keys ServiceKeys,
	setup ProviderSetup,
	settings ServiceSettings,
) (*Services, error) {
	registry, err := serviceRegistry(settings)
	if err != nil {
		return nil, err
	}
	hasher, err := accounts.NewPasswordHasher(ctx)
	if err != nil {
		return nil, fmt.Errorf("password hashing cannot start: %w", err)
	}
	releases, err := providers.NewClaudeReleases(setup.ClaudeReleases)
	if err != nil {
		return nil, fmt.Errorf("the HTTP client cannot start: %w", err)
	}
	syncs := &pagesync.SyncRegistry{}
	vault := providers.NewVault(storage.Control, &keys.Vault, settings.Mode, syncs)
	httpClient := &http.Client{}
	assembly := assembleProviders(vault, storage, setup, httpClient)
	operations := &providers.Operations{}
	streams := pluginStreams(registry)
	return &Services{
		Mode:           settings.Mode,
		Clock:          setup.Clock,
		Control:        storage.Control,
		Conversations:  storage.Conversations,
		Blobs:          storage.Blobs,
		Hasher:         hasher,
		Sessions:       accounts.NewWebSessions(storage.Control),
		Limiter:        accounts.NewLoginLimiter(),
		Email:          accounts.NewEmailChanges(storage.Control, hasher, settings.Mail, keys.EmailCodes),
		Vault:          vault,
		Assembly:       assembly,
		ClaudeReleases: releases,
		CLIInstalls:    &CLIInstalls{},
		Operations:     operations,
		Logins:         providers.NewLoginFlows(assembly, operations, setup.Logins),
		Claims: runners.NewPendingClaims(
			settings.Runners.ClaimsPerMinute,
		),
		Runners:            settings.Runners,
		ConversationTuning: settings.Conversations,
		Pages:              settings.Pages,
		Native:             settings.Native,
		Plugins:            registry,
		UserStreams: hostaccess.NewUserStreams(
			streams,
			settings.Native,
		),
		Cloud:        settings.Cloud,
		PublicURL:    &runners.PublicURL{},
		Lifecycle:    settings.Lifecycle,
		ExposeDomain: settings.ExposeDomain,
		ExposeTuning: settings.Exposes,
		Sync:         syncs,
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

// pluginStreams supplies registered stream operations to host access.
func pluginStreams(registry *plugins.Registry) []hostaccess.StreamDeclaration {
	var streams []hostaccess.StreamDeclaration
	for _, stream := range registry.Streams() {
		streams = append(streams, hostaccess.StreamDeclaration{Name: string(stream.Name), Operation: stream.Operation})
	}
	return streams
}

// assembleProviders connects provider families to the shared catalogs and account services.
func assembleProviders(
	vault *providers.Vault,
	storage *Storage,
	setup ProviderSetup,
	httpClient *http.Client,
) *providers.Assembly {
	models := provider.NewModelsDevClient(httpClient, setup.ModelsDevURL.String(), setup.Clock)
	assembly := providers.NewAssembly(
		vault,
		setup.Families,
		providers.NewAccountQuotas(vault),
		providers.NewModelCatalogCache(storage.Control, setup.Clock),
		providers.NewVendorCatalog(models),
		httpClient,
		setup.Clock,
	)
	return assembly
}

// serviceRegistry validates plugin declarations against the deployment’s native operation catalog.
func serviceRegistry(settings ServiceSettings) (*plugins.Registry, error) {
	registry, err := plugins.NewRegistry(settings.Plugins, func(operation declare.NativeOperation) bool {
		return settings.Native.Serves(operation.Package, []string{operation.Operation})
	})
	if err != nil {
		return nil, fmt.Errorf("the plugins cannot start: %w", err)
	}
	return registry, nil
}
