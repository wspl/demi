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
	// The data directory (`storage.md` § Ownership and layout).
	DataDir string
	// The machine manager's Unix socket (`managed-hosts.md` § Control and
	// ownership): every deployment has Cloud.
	MachinesSocket string
	// Where the listener binds; port 0 picks a free port.
	Address netip.AddrPort
	// Who configures providers (`product.md` § Instance mode).
	Mode webapi.InstanceMode
	// The web app build's directory, served beside the API.
	WebDirectory string
	// The URL runners connect to, which the installers name; without it,
	// the origin an installer was requested from.
	PublicURL *url.URL
	// The domain expose hostnames live under; without it, exposes are
	// unavailable (`expose.md` § Deployment).
	ExposeDomain *expose.Domain
	// The runner releases the installer routes serve; without them, the
	// installers answer 503.
	RunnerReleases string
	// The JSON file that puts the object store in an S3 bucket; without it,
	// the data directory holds it.
	ObjectStore string
	// The instance secret; without it, the one in the data directory, which
	// the first start creates.
	InstanceSecret *InstanceSecret
	// Delivers email verification codes; without it, an email change answers
	// `mail_unavailable`.
	AccountMail accounts.AccountMail
	// Clock supplies wall timestamps; timers use the Go runtime clock.
	Clock core.Clock
	// The provider families entries are assembled with.
	Families *providers.FamilyRegistry
	// The plugins, in their order of registration.
	Plugins []plugin.Factory
	// Where the models.dev document is read.
	ModelsDevURL *url.URL
	// The Claude Code distribution whose newest release the CLI on each
	// Cloud follows (`claude-code.md` § Which version).
	ClaudeReleases *url.URL
	// How long a device login waits for its user, and how long its result
	// is kept.
	Logins providers.LoginTiming
	// How runner connections are timed and pairing is limited.
	Runners usershard.RunnerTuning
	// How conversations are served and their inference limited.
	Conversations usershard.ConversationTuning
	// How the sockets to a page are timed.
	Pages usershard.PageTuning
	// The command packages the conversations' commands bind to.
	Native *runners.NativeCatalog
	// When a conversation's Host resources are reclaimed.
	Lifecycle usershard.LifecycleTuning
	// How the Cloud is run.
	Cloud cloud.CloudTuning
	// How the public relay treats its connections.
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
func NewConfig(dataDir string, address netip.AddrPort, mode webapi.InstanceMode, machinesSocket string) (Config, error) {
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
		DataDir: dataDir, Address: address, Mode: mode, MachinesSocket: machinesSocket,
		Clock: core.SystemClock{}, Families: BuiltinFamilies(), Plugins: plugins,
		ModelsDevURL: models, ClaudeReleases: releases, Logins: providers.DefaultLoginTiming(),
		Runners: usershard.DefaultRunnerTuning(), Conversations: usershard.DefaultConversationTuning(),
		Pages: usershard.DefaultPageTuning(), Native: runners.UnpublishedCatalog(),
		Lifecycle: usershard.DefaultLifecycleTuning(), Cloud: cloud.DefaultTuning(), Exposes: usershard.DefaultExposeTuning(),
	}, nil
}
