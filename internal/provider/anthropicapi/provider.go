package anthropicapi

import (
	"context"
	"net/http"
	"net/url"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// DefaultBaseURL is the base URL of an entry that names none.
const DefaultBaseURL = "https://api.anthropic.com/v1"

// Config is the configuration of an anthropic entry, as the backend decoded it.
type Config struct {
	APIKey provider.Secret
	// The Messages API base with its version prefix, such as
	// `https://api.anthropic.com/v1`; nil for that default. The provider
	// appends `/messages` unless the URL already ends with it.
	BaseURL *url.URL
	Policy  provider.VendorPolicy
}

// Provider is an anthropic entry, shared by every user and request of the entry.
type Provider struct {
	key      provider.Secret
	endpoint string
	policy   provider.VendorPolicy
	clock    types.Clock
}

// New constructs an entry from decoded configuration.
func New(config Config, clock types.Clock) *Provider {
	base := config.BaseURL
	if base == nil {
		base, _ = url.Parse(DefaultBaseURL) // The constant is a valid URL.
	}
	return &Provider{
		key:      config.APIKey.HeaderValue(),
		endpoint: provider.EndpointURL(base, "/messages").String(),
		policy:   config.Policy,
		clock:    clock,
	}
}

// Capabilities returns this API entry's Host requirements.
func (*Provider) Capabilities() provider.Capabilities {
	return provider.Capabilities{}
}

// AuthStatus reports the configured key; the first request establishes vendor acceptance.
func (*Provider) AuthStatus(context.Context) types.AuthState {
	return &types.Authenticated{}
}

// RuntimeState reports that the entry is ready.
func (*Provider) RuntimeState() types.RuntimeState {
	message := "Uses the Anthropic Messages API"
	return &types.RuntimeReady{Message: &message}
}

// ListModels returns the built-in directory without a vendor request.
func (*Provider) ListModels(context.Context) (types.ProviderModelList, error) {
	return directory(), nil
}

// ReadFailure applies the standard reading of HTTP failures.
func (*Provider) ReadFailure(d *types.ProviderErrorDiagnostics, at types.Timestamp) types.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, at)
}

// Quota returns nil because API keys have no subscription quota.
func (*Provider) Quota() *provider.Quota {
	return nil
}

// Accounts returns nil because API keys have no subscription accounts.
func (*Provider) Accounts() provider.SubscriptionAccounts {
	return nil
}

// Runtime creates a session runtime with the owner's HTTP client.
func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.Runtime, error) {
	return &runtime{shared: p, http: env.HTTP}, nil
}

type runtime struct {
	shared *Provider
	http   *http.Client
}

// Fresh returns an independent runtime for another session.
func (r *runtime) Fresh() provider.Runtime {
	return &runtime{shared: r.shared, http: r.http}
}

// Close releases the runtime resources.
func (*runtime) Close(context.Context) error {
	return nil
}

// RequestLimits returns the request limits for the model.
func (*runtime) RequestLimits(model types.Model) provider.RequestLimits {
	return provider.AnthropicRequestLimits(model)
}

var _ provider.Provider = (*Provider)(nil)
