// Package codex implements the ChatGPT subscription's Codex Responses backend.
package codex

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"runtime"
	"strings"
	"sync/atomic"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//go:generate go run github.com/wspl/demi/go/cmd/wiregen
const BackendURL = "https://chatgpt.com/backend-api"
const AuthURL = "https://auth.openai.com"
const SignatureTag = "codex:"
const ClientVersion = "0.153.4"

type TransportMode string

const (
	TransportAuto      TransportMode = "auto"
	TransportSSE       TransportMode = "sse"
	TransportWebSocket TransportMode = "websocket"
)

type Config struct {
	Account                       *string
	BackendURL, AuthURL           url.URL
	Transport                     TransportMode
	HeaderTimeout, ConnectTimeout time.Duration
	StreamIdleTimeout             *time.Duration
}

func NewConfig(account *string) Config {
	backend, _ := url.Parse(BackendURL)
	auth, _ := url.Parse(AuthURL)
	return Config{Account: account, BackendURL: *backend, AuthURL: *auth, Transport: TransportAuto, HeaderTimeout: 20 * time.Second, ConnectTimeout: 10 * time.Second}
}

type Provider struct {
	config                                                     Config
	responsesURL, websocketURL, modelsURL, usageURL, userAgent string
	unreachable                                                atomic.Bool
	auth                                                       *auth
	quota                                                      *provider.ProviderQuota
	accounts                                                   *provider.Accounts
	http                                                       *http.Client
	clock                                                      core.Clock
}

func New(config Config, pool provider.CredentialPool, snapshots provider.QuotaSnapshotStore, client *http.Client, clock core.Clock) (*Provider, error) {
	endpoint := func(base *url.URL, path string) (string, error) {
		value, err := provider.EndpointURL(base, path)
		if err != nil {
			return "", err
		}
		return value.String(), nil
	}
	tokenURL, err := endpoint(&config.AuthURL, "/oauth/token")
	if err != nil {
		return nil, err
	}
	responses, err := codexURL(config.BackendURL, "/responses")
	if err != nil {
		return nil, err
	}
	models, err := codexURL(config.BackendURL, "/models")
	if err != nil {
		return nil, err
	}
	query := models.Query()
	query.Add("client_version", ClientVersion)
	models.RawQuery = query.Encode()
	socket := *responses
	if socket.Scheme == "http" {
		socket.Scheme = "ws"
	} else {
		socket.Scheme = "wss"
	}
	usage, err := endpoint(&config.BackendURL, "/wham/usage")
	if err != nil {
		return nil, err
	}
	p := &Provider{config: config, responsesURL: responses.String(), websocketURL: socket.String(), modelsURL: models.String(), usageURL: usage, userAgent: fmt.Sprintf("demi-codex-provider/0.1.3 (%s; %s)", runtime.GOOS, platformArch()), http: client, clock: clock, auth: &auth{pool: pool, account: config.Account, tokenURL: tokenURL, clock: clock}}
	p.accounts = provider.NewAccounts(pool, &accountKit{http: client, authURL: config.AuthURL, clock: clock}, clock)
	p.quota = provider.NewProviderQuota(&quotaSource{provider: p}, snapshots, clock)
	return p, nil
}
func codexURL(base url.URL, path string) (*url.URL, error) {
	trimmed := strings.TrimRight(base.Path, "/")
	if strings.HasSuffix(trimmed, path) {
		return provider.EndpointURL(&base, path)
	}
	if !strings.HasSuffix(trimmed, "/codex") {
		next, err := provider.EndpointURL(&base, "/codex")
		if err != nil {
			return nil, err
		}
		base = *next
	}
	return provider.EndpointURL(&base, path)
}
func (*Provider) Capabilities() provider.Capabilities { return provider.Capabilities{} }
func (p *Provider) AuthStatus(ctx context.Context) core.AuthState {
	secret, failure := p.auth.stored(ctx)
	if failure != nil {
		return failure.State()
	}
	return core.AuthStateAuthenticated{AccountLabel: new(secret.label().Label)}
}
func (*Provider) RuntimeState() core.RuntimeState {
	return core.RuntimeStateReady{Message: new("Uses the Codex backend with the account's ChatGPT sign-in")}
}
func (*Provider) ReadFailure(d core.ProviderErrorDiagnostics, t core.Timestamp) core.ProviderFailureFacts {
	return ReadFailure(d, t)
}
func (p *Provider) Quota() *provider.ProviderQuota          { return p.quota }
func (p *Provider) Accounts() provider.SubscriptionAccounts { return p.accounts }
func (p *Provider) accountHeaders(secret SecretDocument) http.Header {
	headers := http.Header{"Authorization": {secret.AccessToken.Bearer()}, "Chatgpt-Account-Id": {secret.AccountID}, "User-Agent": {p.userAgent}}
	if accountOf(secret.AccessToken).fedramp || accountOf(secret.IDToken).fedramp {
		headers.Set("X-Openai-Fedramp", "true")
	}
	return headers
}
func (p *Provider) get(ctx context.Context, endpoint string, retry401 bool) (*http.Response, error) {
	var refused *provider.Secret
	for {
		secret, failure := p.auth.credentials(ctx, p.http, refused)
		if failure != nil {
			return nil, failure
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
		if err != nil {
			return nil, fmt.Errorf("Codex request could not be built")
		}
		request.Header = p.accountHeaders(secret)
		request.Header.Set("Accept", "application/json")
		response, err := p.http.Do(request)
		if err != nil {
			return nil, provider.TransportFailure("Codex", err)
		}
		if response.StatusCode == 401 && retry401 && refused == nil {
			response.Body.Close()
			refused = &secret.AccessToken
			continue
		}
		return response, nil
	}
}

func platformArch() string {
	switch runtime.GOARCH {
	case "amd64":
		return "x86_64"
	case "arm64":
		return "aarch64"
	default:
		return runtime.GOARCH
	}
}
