package grokbuild

import (
	"context"
	"net/http"
	"net/url"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// ProxyURL is the product's chat proxy.
const ProxyURL = "https://cli-chat-proxy.grok.com/v1"

// IssuerURL is the product's login issuer.
const IssuerURL = "https://auth.x.ai"
const label = "Grok Build"

// Config configures one account of a grok-build entry.
type Config struct {
	// The account the provider stands for; nil only for a provider built to log in.
	Account *string
	// The chat proxy, https://cli-chat-proxy.grok.com/v1 in the product.
	ProxyURL *url.URL
	// The login issuer; an existing account refreshes at its stored issuer.
	IssuerURL *url.URL
}

// NewConfig returns the product configuration for account, or nil for login.
func NewConfig(account *string) Config {
	proxy, _ := url.Parse(ProxyURL)
	issuer, _ := url.Parse(IssuerURL + "/") // Constant URLs are valid.
	return Config{Account: account, ProxyURL: proxy, IssuerURL: issuer}
}

// Provider is a grok-build entry's provider for one account.
type Provider struct {
	auth               *auth
	quota              *provider.Quota
	accounts           *provider.Accounts
	http               *http.Client
	clock              core.Clock
	chatURL, modelsURL *url.URL
}

// New connects an account's provider to the entry's credentials and quota store.
func New(config Config, pool provider.CredentialPool, snapshots provider.QuotaSnapshotStore, httpClient *http.Client, clock core.Clock) *Provider {
	var account *string
	if config.Account != nil {
		id := *config.Account
		account = &id
	}
	a := &auth{pool: pool, account: account, clock: clock}
	issuer := *config.IssuerURL
	if issuer.Path == "" {
		issuer.Path = "/"
	}
	user := provider.EndpointURL(config.ProxyURL, "/user")
	kit := &loginKit{http: httpClient, issuer: &issuer, userURL: user, clock: clock}
	probeUser := *user
	probeUser.RawQuery = "include=subscription"
	billing := provider.EndpointURL(config.ProxyURL, "/billing")
	billing.RawQuery = "format=credits"
	q := &quotaSource{auth: a, http: httpClient, userURL: &probeUser, billingURL: billing}
	return &Provider{auth: a, quota: provider.NewQuota(q, snapshots, clock), accounts: provider.NewAccounts(pool, kit, clock), http: httpClient, clock: clock, chatURL: provider.EndpointURL(config.ProxyURL, "/chat/completions"), modelsURL: provider.EndpointURL(config.ProxyURL, "/models")}
}

// Capabilities reports that inference requires no Host process.
func (*Provider) Capabilities() provider.Capabilities { return provider.Capabilities{} }

// AuthStatus reads the stored account without refreshing it.
func (p *Provider) AuthStatus(ctx context.Context) core.AuthState {
	s, failure := p.auth.stored(ctx)
	if failure != nil {
		return failure.State()
	}
	name := s.label().Label
	return &core.Authenticated{AccountLabel: &name}
}

// RuntimeState reports the chat proxy transport.
func (*Provider) RuntimeState() core.RuntimeState {
	message := "Uses the Grok Build chat proxy with the account's sign-in"
	return &core.RuntimeReady{Message: &message}
}

// ReadFailure reads the standard HTTP retry facts.
func (*Provider) ReadFailure(d *core.ProviderErrorDiagnostics, at core.Timestamp) core.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, at)
}

// Quota returns this account's quota.
func (p *Provider) Quota() *provider.Quota { return p.quota }

// Accounts returns the entry's subscription account operations.
func (p *Provider) Accounts() provider.SubscriptionAccounts { return p.accounts }

// Runtime builds a session runtime using its owner's HTTP client.
func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.Runtime, error) {
	return &runtime{shared: p, http: env.HTTP}, nil
}

type runtime struct {
	shared *Provider
	http   *http.Client
}

func (r *runtime) Fresh() provider.Runtime                       { return &runtime{shared: r.shared, http: r.http} }
func (*runtime) Close(context.Context) error                     { return nil }
func (*runtime) RequestLimits(core.Model) provider.RequestLimits { return provider.RequestLimits{} }

var _ provider.Provider = (*Provider)(nil)
