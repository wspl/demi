package claudecode

import (
	"context"
	"fmt"
	"net/http"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

const family = "Claude Code"

// Config is the configuration of a `claude-code` entry's provider for one account.
type Config struct {
	// The entry's id, which names the entry in the logs of its runs.
	ID string
	// The entry's label, which names the provider in its refusals.
	DisplayName string
	// The account the provider stands for; nil only for a provider built
	// to add the entry's first account.
	Account *string
	// The OAuth usage endpoint the quota probe reads,
	// `https://api.anthropic.com/api/oauth/usage` in the product.
	UsageURL string
}

// NewConfig returns the product's configuration of an entry's provider for account.
func NewConfig(id, displayName string, account *string) Config {
	return Config{
		ID:          id,
		DisplayName: displayName,
		Account:     account,
		UsageURL:    "https://api.anthropic.com/api/oauth/usage",
	}
}

// Provider connects one account to runtimes whose placement chooses the machine.
type Provider struct {
	config   Config
	pool     provider.CredentialPool
	accounts *provider.Accounts
	quota    *provider.Quota
	models   *provider.ModelsDevClient
}

// New builds a provider over the entry's pool and account's quota snapshots.
func New(
	config Config,
	pool provider.CredentialPool,
	snapshots provider.QuotaSnapshotStore,
	models *provider.ModelsDevClient,
	httpClient *http.Client,
	clock types.Clock,
) *Provider {
	p := &Provider{
		config:   config,
		pool:     pool,
		models:   models,
		accounts: provider.NewAccounts(pool, accountKit{}, clock),
	}
	p.quota = provider.NewQuota(&quotaSource{owner: p, http: httpClient}, snapshots, clock)
	return p
}

// Capabilities reports that inference requires a process placement.
func (*Provider) Capabilities() provider.Capabilities {
	return provider.Capabilities{ProcessHost: true}
}

// AuthStatus reads the stored account without inference.
func (p *Provider) AuthStatus(ctx context.Context) types.AuthState {
	secret, err := p.stored(ctx)
	if err != nil {
		return provider.AccountAuthFailure(family, err).State()
	}
	label := secret.label().Label
	return &types.Authenticated{AccountLabel: &label}
}

// RuntimeState reports that process availability is known only at start.
func (*Provider) RuntimeState() types.RuntimeState {
	message := "Claude Code runs on the user's Cloud when a request needs it"
	return &types.RuntimeUnknown{Message: &message}
}

// ReadFailure reads the standard HTTP facts when a record supplies them.
func (*Provider) ReadFailure(d *types.ProviderErrorDiagnostics, at types.Timestamp) types.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, at)
}

// Quota returns the account's usage probe and observations.
func (p *Provider) Quota() *provider.Quota { return p.quota }

// Accounts returns shared subscription operations for setup tokens.
func (p *Provider) Accounts() provider.SubscriptionAccounts { return p.accounts }

// Runtime refuses because only the backend can choose a process machine.
func (p *Provider) Runtime(provider.RuntimeEnv) (provider.Runtime, error) {
	return nil, fmt.Errorf("%s needs a Host that runs processes", p.config.DisplayName)
}

// ProcessRuntime builds a session runtime over placement.
func (p *Provider) ProcessRuntime(placement Placement) provider.Runtime {
	return &runtime{owner: p, placement: placement}
}

// Site describes where a new CLI process runs on the machine the placement chose.
type Site struct {
	// The absolute path of Demi's CLI executable there.
	Executable string
	// The process's working directory, `~/.demi/claude/run`.
	RunDir string
	// The CLI's configuration home, `~/.demi/claude/config`.
	ConfigDir string
}

// Placement starts a CLI process on a machine it chooses and readies.
type Placement interface {
	Start(context.Context, func(Site) host.SpawnRequest) (*host.StartedProcess, error)
}

var _ provider.Provider = (*Provider)(nil)
