package backend

import (
	"net/netip"
	"net/url"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// Config is everything Start needs: the configured values, and the parts a
// test replaces. It is immutable after Start; shared registries and factories
// retain their own synchronization and lifecycle contracts.
type Config struct {
	// objectStoreSet preserves an explicitly empty CLI path.
	objectStoreSet bool
	// DataDir is the data directory (`storage.md` § Ownership and layout).
	DataDir string
	// MachinesSocket is the machine manager's Unix socket (`managed-hosts.md` § Control and
	// ownership): every deployment has Cloud.
	MachinesSocket string
	// Address is where the listener binds; port 0 picks a free port.
	Address netip.AddrPort
	// Mode is who configures providers (`product.md` § Instance mode).
	Mode webapi.InstanceMode
	// WebDirectory is the web app build's directory, served beside the API.
	WebDirectory string
	// PublicURL is the URL runners connect to, which the installers name; without it,
	// the origin an installer was requested from.
	PublicURL *url.URL
	// ExposeDomain is the domain expose hostnames live under; without it, exposes are
	// unavailable (`expose.md` § Deployment).
	ExposeDomain *expose.Domain
	// RunnerReleases is the runner releases the installer routes serve; without them, the
	// installers answer 503.
	RunnerReleases string
	// ObjectStore is the JSON file that puts the object store in an S3 bucket; without it,
	// the data directory holds it.
	ObjectStore string
	// InstanceSecret is the instance secret; without it, the one in the data directory, which
	// the first start creates.
	InstanceSecret *InstanceSecret
	// AccountMail delivers email verification codes; without it, an email change answers
	// `mail_unavailable`.
	AccountMail accounts.AccountMail
	// Clock supplies wall timestamps; timers use the Go runtime clock.
	Clock core.Clock
	// Families supplies the provider families entries are assembled with.
	Families *providers.FamilyRegistry
	// Plugins lists the plugins in their order of registration.
	Plugins []plugin.Factory
	// ModelsDevURL is where the models.dev document is read.
	ModelsDevURL *url.URL
	// ClaudeReleases is the Claude Code distribution whose newest release the CLI on each
	// Cloud follows (`claude-code.md` § Which version).
	ClaudeReleases *url.URL
	// Logins is how long a device login waits for its user, and how long its result
	// is kept.
	Logins providers.LoginTiming
	// Runners is how runner connections are timed and pairing is limited.
	Runners usershard.RunnerTuning
	// Conversations is how conversations are served and their inference limited.
	Conversations usershard.ConversationTuning
	// Pages is how the sockets to a page are timed.
	Pages usershard.PageTuning
	// Native supplies the command packages the conversations' commands bind to.
	Native *runners.NativeCatalog
	// Lifecycle is when a conversation's Host resources are reclaimed.
	Lifecycle usershard.LifecycleTuning
	// Cloud is how the Cloud is run.
	Cloud cloud.CloudTuning
	// Exposes is how the public relay treats its connections.
	Exposes usershard.ExposeTuning
	// Hooks supplies optional test synchronization points before any flow starts.
	Hooks usershard.FlowHooks
	// ObserveObjects optionally wraps object operations for test measurements.
	// The wrapper borrows the store; backend retains and closes its owner.
	ObserveObjects func(blobs.Objects) blobs.Objects
}

// NewConfig returns a configuration on the system clock with the built-in
// families and plugins, the published models.dev document, no web directory
// and no mail sender, whose Clouds the manager at machinesSocket runs.
// Plugin declaration failures are returned instead of panicking.
func NewConfig(
	dataDir string,
	address netip.AddrPort,
	mode webapi.InstanceMode,
	machinesSocket string,
) (Config, error) {
	plugins, err := BuiltinPlugins()
	if err != nil {
		return Config{}, err
	}
	models, err := url.Parse(provider.ModelsDevURL)
	if err != nil {
		return Config{}, err
	}
	releases, err := url.Parse(providers.DefaultReleasesURL)
	if err != nil {
		return Config{}, err
	}
	return Config{
		DataDir:        dataDir,
		Address:        address,
		Mode:           mode,
		MachinesSocket: machinesSocket,
		Clock:          core.SystemClock{},
		Families:       BuiltinFamilies(),
		Plugins:        plugins,
		ModelsDevURL:   models,
		ClaudeReleases: releases,
		Logins:         providers.DefaultLoginTiming(),
		Runners:        usershard.DefaultRunnerTuning(),
		Conversations:  usershard.DefaultConversationTuning(),
		Pages:          usershard.DefaultPageTuning(),
		Native:         runners.UnpublishedCatalog(),
		Lifecycle:      usershard.DefaultLifecycleTuning(),
		Cloud:          cloud.DefaultTuning(),
		Exposes:        usershard.DefaultExposeTuning(),
	}, nil
}
