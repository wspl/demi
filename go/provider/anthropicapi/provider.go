// Package anthropicapi implements the Anthropic Messages API.
package anthropicapi

import (
	"context"
	"io"
	"iter"
	"net/http"
	"net/url"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//go:generate go run github.com/wspl/demi/go/cmd/wiregen
const DefaultBaseURL = "https://api.anthropic.com/v1"

type Config struct {
	APIKey  provider.Secret
	BaseURL *url.URL
}
type Provider struct {
	apiKey, url string
	clock       core.Clock
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
	endpoint, err := provider.EndpointURL(base, "/messages")
	if err != nil {
		return nil, err
	}
	return &Provider{apiKey: config.APIKey.Expose(), url: endpoint.String(), clock: clock}, nil
}
func (*Provider) Capabilities() provider.Capabilities       { return provider.Capabilities{} }
func (*Provider) AuthStatus(context.Context) core.AuthState { return core.AuthStateAuthenticated{} }
func (*Provider) RuntimeState() core.RuntimeState {
	message := "Uses the Anthropic Messages API"
	return core.RuntimeStateReady{Message: &message}
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
func (*runtime) RequestLimits(model core.Model) provider.RequestLimits {
	return provider.AnthropicRequestLimits(model)
}
func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
	vendor := provider.Vendor{Label: "Anthropic", Reader: provider.ReadHTTPFailure, Clock: r.shared.clock}
	return provider.HTTPRun(ctx, r.http, r.shared.url, http.Header{"X-Api-Key": {r.shared.apiKey}, "Anthropic-Version": {"2023-06-01"}}, func() []byte { return body(request) }, vendor, func(ctx context.Context, body io.Reader) iter.Seq[provider.ProviderEvent] {
		return mapSSE(ctx, body, vendor)
	})
}
func (*Provider) ListModels(context.Context) (core.ProviderModelList, error) {
	models := []core.ProviderModel{}
	for _, entry := range []struct {
		id, name string
		output   uint32
		wide     bool
	}{{"claude-opus-4-8", "Claude Opus 4.8", 128000, true}, {"claude-opus-4-7", "Claude Opus 4.7", 128000, true}, {"claude-opus-4-6", "Claude Opus 4.6", 128000, false}, {"claude-sonnet-4-6", "Claude Sonnet 4.6", 64000, false}, {"claude-fable-5", "Claude Fable 5", 128000, true}} {
		efforts := []string{"low", "medium", "high", "max"}
		if entry.wide {
			efforts = []string{"low", "medium", "high", "xhigh", "max"}
		}
		models = append(models, core.ProviderModel{ID: entry.id, DisplayName: entry.name, ContextWindow: new(uint32(1000000)), OutputLimit: &entry.output, SupportsTools: new(true), SupportsAttachments: new(true), SupportsVideo: new(false), SupportsReasoning: new(true), SupportedThinkingEfforts: &efforts, CanDisableThinking: new(false), ServiceTiers: []core.ServiceTier{}})
	}
	return core.ProviderModelList{Models: models, DefaultModelID: new("claude-opus-4-8"), Warnings: []string{}, SourceFetchedAt: core.UnixEpoch}, nil
}
