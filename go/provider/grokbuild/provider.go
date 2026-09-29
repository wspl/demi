// Package grokbuild implements the Grok Build subscription chat proxy.
package grokbuild

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"iter"
	"net/http"
	"net/url"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//go:generate go run github.com/wspl/demi/go/cmd/wiregen
const ProxyURL = "https://cli-chat-proxy.grok.com/v1"
const IssuerURL = "https://auth.x.ai"
const ClientVersion = "1.0.5"
const label = "Grok Build"

type Config struct {
	Account             *string
	ProxyURL, IssuerURL url.URL
}

func NewConfig(account *string) Config {
	proxy, _ := url.Parse(ProxyURL)
	issuer, _ := url.Parse(IssuerURL)
	return Config{Account: account, ProxyURL: *proxy, IssuerURL: *issuer}
}

type Provider struct {
	config                                  Config
	pool                                    provider.CredentialPool
	clock                                   core.Clock
	http                                    *http.Client
	chatURL, modelsURL, userURL, billingURL string
	quota                                   *provider.ProviderQuota
	accounts                                *provider.Accounts
}

func New(config Config, pool provider.CredentialPool, snapshots provider.QuotaSnapshotStore, client *http.Client, clock core.Clock) (*Provider, error) {
	p := &Provider{config: config, pool: pool, clock: clock, http: client}
	for path, dest := range map[string]*string{"/chat/completions": &p.chatURL, "/models": &p.modelsURL, "/user": &p.userURL, "/billing": &p.billingURL} {
		endpoint, err := provider.EndpointURL(&config.ProxyURL, path)
		if err != nil {
			return nil, err
		}
		*dest = endpoint.String()
	}
	p.quota = provider.NewProviderQuota(&quotaSource{p: p}, snapshots, clock)
	p.accounts = provider.NewAccounts(pool, &accountKit{p: p}, clock)
	return p, nil
}
func (*Provider) Capabilities() provider.Capabilities { return provider.Capabilities{} }
func (p *Provider) AuthStatus(ctx context.Context) core.AuthState {
	secret, failure := p.stored(ctx)
	if failure != nil {
		return failure.State()
	}
	return core.AuthStateAuthenticated{AccountLabel: new(secret.label().Label)}
}
func (*Provider) RuntimeState() core.RuntimeState {
	return core.RuntimeStateReady{Message: new("Uses the Grok Build chat proxy with the account's sign-in")}
}
func (*Provider) ReadFailure(d core.ProviderErrorDiagnostics, t core.Timestamp) core.ProviderFailureFacts {
	return provider.ReadHTTPFailure(d, t)
}
func (p *Provider) Quota() *provider.ProviderQuota          { return p.quota }
func (p *Provider) Accounts() provider.SubscriptionAccounts { return p.accounts }
func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.ProviderRuntime, error) {
	return &runtimeProvider{p: p, http: env.HTTP}, nil
}

type runtimeProvider struct {
	p    *Provider
	http *http.Client
}

func (r *runtimeProvider) Fresh() provider.ProviderRuntime {
	return &runtimeProvider{p: r.p, http: r.http}
}
func (*runtimeProvider) Close(context.Context) error { return nil }
func (*runtimeProvider) RequestLimits(core.Model) provider.RequestLimits {
	return provider.RequestLimits{}
}
func (r *runtimeProvider) Run(ctx context.Context, request provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
	return func(yield func(provider.ProviderEvent) bool) {
		if ctx.Err() != nil {
			return
		}
		body, failure := provider.EncodeBody(label, func() []byte { return encodeRequest(request) })
		if ctx.Err() != nil {
			return
		}
		if failure != nil {
			yield(provider.FailureEvent{Failure: *failure})
			return
		}
		var refused *provider.Secret
		for {
			secret, failure := r.p.credentials(ctx, r.http, refused)
			if ctx.Err() != nil {
				return
			}
			if failure != nil {
				yield(provider.FailureEvent{Failure: failure.Failure()})
				return
			}
			response, err := send(ctx, r.http, http.MethodPost, r.p.chatURL, inferenceHeaders(secret, request), body)
			if ctx.Err() != nil {
				if response != nil {
					response.Body.Close()
				}
				return
			}
			if err != nil {
				yield(provider.FailureEvent{Failure: provider.TransportFailure(label, err)})
				return
			}
			r.p.quota.Observe(provider.Observation{Status: uint16(response.StatusCode), Headers: response.Header})
			if response.StatusCode == 401 && refused == nil {
				response.Body.Close()
				refused = &secret.AccessToken
				continue
			}
			defer response.Body.Close()
			if response.StatusCode < 200 || response.StatusCode >= 300 {
				failure := provider.HTTPFailure(response, label, provider.ReadHTTPFailure, r.p.clock)
				if ctx.Err() == nil {
					yield(provider.FailureEvent{Failure: failure})
				}
				return
			}
			for event := range provider.MapChatSSE(ctx, response.Body, provider.Vendor{Label: label, Reader: provider.ReadHTTPFailure, Clock: r.p.clock}) {
				if ctx.Err() != nil || !yield(event) {
					return
				}
			}
			return
		}
	}
}

// send attaches the account or login headers to one Grok request.
func send(ctx context.Context, client *http.Client, method, endpoint string, headers http.Header, body []byte) (*http.Response, error) {
	request, err := http.NewRequestWithContext(ctx, method, endpoint, bytes.NewReader(body))
	if err != nil {
		return nil, fmt.Errorf("Grok request could not be built")
	}
	request.Header = headers.Clone()
	return client.Do(request)
}
func readResponse(response *http.Response) ([]byte, error) {
	defer response.Body.Close()
	return io.ReadAll(response.Body)
}
