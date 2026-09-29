// Package claudecode implements Claude Code subscription accounts and CLI integration.
package claudecode

import (
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/url"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//go:generate go run github.com/wspl/demi/go/cmd/wiregen
const UsageURL = "https://api.anthropic.com/api/oauth/usage"

type Config struct {
	ID, DisplayName string
	Account         *string
	UsageURL        url.URL
}

func NewConfig(id, displayName string, account *string) Config {
	usage, _ := url.Parse(UsageURL)
	return Config{ID: id, DisplayName: displayName, Account: account, UsageURL: *usage}
}

type Provider struct {
	config   Config
	pool     provider.CredentialPool
	models   *provider.ModelsDevClient
	http     *http.Client
	quota    *provider.ProviderQuota
	accounts *provider.Accounts
}

func New(config Config, pool provider.CredentialPool, snapshots provider.QuotaSnapshotStore, models *provider.ModelsDevClient, http *http.Client, clock core.Clock) *Provider {
	p := &Provider{config: config, pool: pool, models: models, http: http}
	p.accounts = provider.NewAccounts(pool, accountKit{}, clock)
	p.quota = provider.NewProviderQuota(&quotaSource{p: p}, snapshots, clock)
	return p
}
func (*Provider) Capabilities() provider.Capabilities {
	return provider.Capabilities{ProcessHost: true}
}
func (p *Provider) AuthStatus(ctx context.Context) core.AuthState {
	secret, failure := p.stored(ctx)
	if failure != nil {
		return failure.State()
	}
	return core.AuthStateAuthenticated{AccountLabel: new(secret.label().Label)}
}
func (*Provider) RuntimeState() core.RuntimeState {
	return core.RuntimeStateUnknown{Message: new("Claude Code runs on the user's Cloud when a request needs it")}
}
func (*Provider) ReadFailure(d core.ProviderErrorDiagnostics, t core.Timestamp) core.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, t)
}
func (p *Provider) Quota() *provider.ProviderQuota          { return p.quota }
func (p *Provider) Accounts() provider.SubscriptionAccounts { return p.accounts }

func (p *Provider) Runtime(provider.RuntimeEnv) (provider.ProviderRuntime, error) {
	return nil, &provider.ProcessHostRequired{Provider: p.config.DisplayName}
}

//demi:wire
type SecretDocument struct {
	AccessToken provider.Secret `json:"accessToken" check:"func=provider.Validate"`
}

func (s SecretDocument) JSON() ([]byte, error) { return encode(s) }
func (s SecretDocument) label() provider.AccountLabel {
	digest := sha256.Sum256([]byte(s.AccessToken.Expose()))
	hex := fmt.Sprintf("%x", digest[:8])
	return provider.AccountLabel{Label: "claude-" + hex[8:], IdentityKey: new("token:" + hex)}
}
func (p *Provider) stored(ctx context.Context) (SecretDocument, *provider.AuthFailure) {
	if p.config.Account == nil {
		return SecretDocument{}, &provider.AuthFailure{Family: "Claude Code", Reason: provider.AuthReasonMissing}
	}
	stored, err := provider.ReadSecret(ctx, p.pool.Document(*p.config.Account), func(raw []byte) (SecretDocument, error) { return provider.DecodeSecret(raw, decode[SecretDocument]) })
	if err != nil {
		return SecretDocument{}, provider.AccountFailure("Claude Code", err)
	}
	return stored.Secret, nil
}

type accountKit struct{}

func (accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Add: true}
}
func (accountKit) Login(context.Context, func(core.LoginPending)) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrLoginUnsupported
}
func (accountKit) Add(input provider.AddAccount) (provider.NewAccount, error) {
	secret := SecretDocument{AccessToken: input.SetupToken}
	raw, err := secret.JSON()
	return provider.NewAccount{Secret: string(raw), Label: secret.label()}, err
}
