// Package openaiapi implements OpenAI Responses and compatible Chat Completions.
package openaiapi

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
const DefaultBaseURL = "https://api.openai.com/v1"
const SignatureTag = "openai:"

type VendorPolicy struct{ PassBackReasoningContent, ReplayAssistantStatus bool }
type Config struct {
	APIKey  provider.Secret
	BaseURL *url.URL
	Wire    core.WireAPI
	Policy  VendorPolicy
}
type Provider struct {
	authorization, url string
	wire               core.WireAPI
	policy             VendorPolicy
	clock              core.Clock
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
	path := "/responses"
	if config.Wire == core.WireAPIChatCompletions {
		path = "/chat/completions"
	}
	endpoint, err := provider.EndpointURL(base, path)
	if err != nil {
		return nil, err
	}
	return &Provider{authorization: config.APIKey.Bearer(), url: endpoint.String(), wire: config.Wire, policy: config.Policy, clock: clock}, nil
}
func (*Provider) Capabilities() provider.Capabilities       { return provider.Capabilities{} }
func (*Provider) AuthStatus(context.Context) core.AuthState { return core.AuthStateAuthenticated{} }
func (p *Provider) RuntimeState() core.RuntimeState {
	message := "Uses the OpenAI Responses API"
	if p.wire == core.WireAPIChatCompletions {
		message = "Uses the OpenAI Chat Completions API"
	}
	return core.RuntimeStateReady{Message: &message}
}
func (*Provider) ListModels(context.Context) (core.ProviderModelList, error) { return directory(), nil }
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
	return provider.OpenAIRequestLimits()
}
func (r *runtime) Run(ctx context.Context, inference provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
	vendor := provider.Vendor{Label: "OpenAI", Reader: provider.ReadHTTPFailure, Clock: r.shared.clock}
	return provider.HTTPRun(ctx, r.http, r.shared.url, http.Header{"Authorization": {r.shared.authorization}}, func() []byte {
		if r.shared.wire == core.WireAPIChatCompletions {
			return chatBody(inference, r.shared.policy)
		}
		return responsesBody(inference, r.shared.policy)
	}, vendor, func(ctx context.Context, body io.Reader) iter.Seq[provider.ProviderEvent] {
		if r.shared.wire == core.WireAPIChatCompletions {
			return provider.MapChatSSE(ctx, body, vendor)
		}
		return provider.MapResponsesSSE(ctx, body, vendor, SignatureTag)
	})
}
