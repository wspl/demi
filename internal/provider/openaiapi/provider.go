package openaiapi

import (
	"context"
	"net/url"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

const signatureTag = "openai:"

// DefaultBaseURL is the base URL of an entry that names none.
const DefaultBaseURL = "https://api.openai.com/v1"

// Config is the configuration of an openai entry, as the backend decoded it.
type Config struct {
	APIKey provider.Secret
	// BaseURL is the API's base; nil selects DefaultBaseURL. The request path
	// is appended unless the URL already ends with it.
	BaseURL *url.URL
	Wire    types.WireAPI
	Policy  provider.VendorPolicy
}

// Provider is an openai entry, shared by every user and request of the entry.
type Provider struct {
	authorization provider.Secret
	url           *url.URL
	wire          types.WireAPI
	policy        provider.VendorPolicy
	clock         types.Clock
}

// New constructs an entry from its decoded configuration.
func New(config Config, clock types.Clock) *Provider {
	base := config.BaseURL
	if base == nil {
		base, _ = url.Parse(DefaultBaseURL) // The fixed default is a valid URL.
	}
	path := "/responses"
	if config.Wire == types.WireAPIChatCompletions {
		path = "/chat/completions"
	}
	return &Provider{
		authorization: config.APIKey.Bearer(),
		url:           provider.EndpointURL(base, path),
		wire:          config.Wire,
		policy:        config.Policy,
		clock:         clock,
	}
}

// Capabilities describes the entry's Host requirements.
func (*Provider) Capabilities() provider.Capabilities {
	return provider.Capabilities{}
}

// AuthStatus reports the entry's key; the vendor validates it on the first request.
func (*Provider) AuthStatus(context.Context) types.AuthState {
	return &types.Authenticated{}
}

// RuntimeState describes the configured wire.
func (p *Provider) RuntimeState() types.RuntimeState {
	message := "Uses the OpenAI Responses API"
	if p.wire == types.WireAPIChatCompletions {
		message = "Uses the OpenAI Chat Completions API"
	}
	return &types.RuntimeReady{Message: &message}
}

// ListModels returns the built-in directory without a request.
func (*Provider) ListModels(context.Context) (types.ProviderModelList, error) {
	return directory(), nil
}

// ReadFailure reads the standard HTTP retry time.
func (*Provider) ReadFailure(d *types.ProviderErrorDiagnostics, at types.Timestamp) types.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, at)
}

// Quota returns nil: this entry has no subscription quota.
func (*Provider) Quota() *provider.Quota {
	return nil
}

// Accounts returns nil: this entry has no subscription accounts.
func (*Provider) Accounts() provider.SubscriptionAccounts {
	return nil
}

// Runtime creates a stateless session runtime using the owner's HTTP client.
func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.Runtime, error) {
	return &runtime{shared: p, http: env.HTTP}, nil
}

var _ provider.Provider = (*Provider)(nil)
