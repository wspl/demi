// Package google implements Gemini's native streaming generateContent API.
package google

import (
	"context"
	"io"
	"iter"
	"net/http"
	"net/url"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//go:generate go run github.com/wspl/demi/go/cmd/wiregen
const DefaultBaseURL = "https://generativelanguage.googleapis.com/v1beta"
const SignatureTag = "google:"

type Config struct {
	APIKey  provider.Secret
	BaseURL *url.URL
}
type Provider struct {
	apiKey string
	base   url.URL
	clock  core.Clock
}

func New(config Config, clock core.Clock) (*Provider, error) {
	base := config.BaseURL
	if base == nil {
		parsed, err := url.Parse(DefaultBaseURL)
		if err != nil {
			return nil, err
		}
		base = parsed
	}
	return &Provider{apiKey: config.APIKey.Expose(), base: *base, clock: clock}, nil
}
func (*Provider) Capabilities() provider.Capabilities       { return provider.Capabilities{} }
func (*Provider) AuthStatus(context.Context) core.AuthState { return core.AuthStateAuthenticated{} }
func (*Provider) RuntimeState() core.RuntimeState {
	return core.RuntimeStateReady{Message: new("Uses the Gemini generateContent API")}
}
func (*Provider) ReadFailure(d core.ProviderErrorDiagnostics, t core.Timestamp) core.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, t)
}
func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.ProviderRuntime, error) {
	return &runtime{shared: p, http: env.HTTP}, nil
}

type runtime struct {
	shared *Provider
	http   *http.Client
}

func (r *runtime) Fresh() provider.ProviderRuntime { return &runtime{shared: r.shared, http: r.http} }
func (*runtime) Close(context.Context) error       { return nil }
func (*runtime) RequestLimits(core.Model) provider.RequestLimits {
	return provider.RequestLimits{BodyBytes: new(uint64(20000000)), Images: new(uint32(3600))}
}
func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
	endpoint := r.shared.base
	endpoint.RawPath = strings.TrimSuffix(endpoint.EscapedPath(), "/") + "/models/" + url.PathEscape(request.ModelID+":streamGenerateContent")
	endpoint.Path, _ = url.PathUnescape(endpoint.RawPath) // Constructed from valid escaped path segments.
	endpoint.RawQuery = "alt=sse"
	vendor := provider.Vendor{Label: "Google", Reader: provider.ReadHTTPFailure, Clock: r.shared.clock}
	return provider.HTTPRun(ctx, r.http, endpoint.String(), http.Header{"X-Goog-Api-Key": {r.shared.apiKey}}, func() []byte { return body(request) }, vendor, func(ctx context.Context, body io.Reader) iter.Seq[provider.ProviderEvent] {
		return mapSSE(ctx, body, vendor)
	})
}
func (*Provider) ListModels(context.Context) (core.ProviderModelList, error) {
	models := []core.ProviderModel{}
	for _, entry := range []struct{ id, name string }{{"gemini-3.6-flash", "Gemini 3.6 Flash"}, {"gemini-3.5-flash", "Gemini 3.5 Flash"}, {"gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"}, {"gemini-2.5-flash", "Gemini 2.5 Flash"}} {
		efforts := []string{"low", "medium", "high", "xhigh", "max"}
		models = append(models, core.ProviderModel{ID: entry.id, DisplayName: entry.name, ContextWindow: new(uint32(1048576)), OutputLimit: new(uint32(65536)), SupportsTools: new(true), SupportsAttachments: new(true), SupportsVideo: new(true), SupportsReasoning: new(true), SupportedThinkingEfforts: &efforts, DefaultThinkingEffort: new("medium"), ServiceTiers: []core.ServiceTier{}})
	}
	return core.ProviderModelList{Models: models, DefaultModelID: new("gemini-3.6-flash"), Warnings: []string{}, SourceFetchedAt: core.UnixEpoch}, nil
}
