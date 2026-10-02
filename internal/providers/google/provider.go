package google

import (
	"context"
	"net/http"
	"net/url"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// DefaultBaseURL is the API base used when Config.BaseURL is nil.
const DefaultBaseURL = "https://generativelanguage.googleapis.com/v1beta"

// Config is the configuration of a google entry, as the backend decoded it.
type Config struct {
	APIKey provider.Secret
	// The API's base with its version, such as
	// `https://generativelanguage.googleapis.com/v1beta`; nil for that
	// default.
	BaseURL *url.URL
}

// Provider is a google entry, shared by every user and request of the entry.
type Provider struct {
	key   provider.Secret
	base  url.URL
	clock core.Clock
}

// New constructs a provider from a decoded configuration and its wall clock.
func New(config Config, clock core.Clock) *Provider {
	base := config.BaseURL
	if base == nil {
		base, _ = url.Parse(DefaultBaseURL) // The fixed default is a valid absolute HTTPS URL.
	}
	return &Provider{key: config.APIKey.HeaderValue(), base: *base, clock: clock}
}

// Capabilities describes this API provider's Host requirements.
func (*Provider) Capabilities() provider.Capabilities { return provider.Capabilities{} }

// AuthStatus reports the configured key; acceptance is checked by the vendor.
func (*Provider) AuthStatus(context.Context) core.AuthState { return &core.Authenticated{} }

// RuntimeState reports the native API transport.
func (*Provider) RuntimeState() core.RuntimeState {
	message := "Uses the Gemini generateContent API"
	return &core.RuntimeReady{Message: &message}
}

// ListModels returns the built-in directory without a network request.
func (*Provider) ListModels(context.Context) (core.ProviderModelList, error) { return directory(), nil }

// ReadFailure applies the standard HTTP failure reading.
func (*Provider) ReadFailure(d *core.ProviderErrorDiagnostics, at core.Timestamp) core.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, at)
}

// Quota returns nil because this API has no quota probe.
func (*Provider) Quota() *provider.Quota { return nil }

// Accounts returns nil because an API key has no subscription account pool.
func (*Provider) Accounts() provider.SubscriptionAccounts { return nil }

// Runtime creates a session runtime using the owner's HTTP client.
func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.Runtime, error) {
	return &runtime{shared: p, http: env.HTTP}, nil
}

func (p *Provider) streamURL(model string) string {
	u := p.base
	escaped := strings.TrimSuffix(u.EscapedPath(), "/") + "/models/" + url.PathEscape(model+":streamGenerateContent")
	u.Path, _ = url.PathUnescape(escaped) // EscapedPath and PathEscape always produce valid escapes.
	u.RawPath = escaped
	u.RawQuery = "alt=sse"
	return u.String()
}

type runtime struct {
	shared *Provider
	http   *http.Client
}

func (r *runtime) Fresh() provider.Runtime   { return &runtime{shared: r.shared, http: r.http} }
func (*runtime) Close(context.Context) error { return nil }
func (*runtime) RequestLimits(core.Model) provider.RequestLimits {
	body, images := uint64(20_000_000), uint32(3600)
	return provider.RequestLimits{BodyBytes: &body, Images: &images}
}

var _ provider.Provider = (*Provider)(nil)
