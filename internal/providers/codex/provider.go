package codex

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/version"
)

var userAgent = codexUserAgent()

// codexUserAgent is demi-codex-provider/<release> (<os>; <arch>), with Go's darwin, amd64 and arm64 spelled macos,
// x86_64 and aarch64.
func codexUserAgent() string {
	osName := runtime.GOOS
	if osName == "darwin" {
		osName = "macos"
	}
	architecture := runtime.GOARCH
	switch architecture {
	case "amd64":
		architecture = "x86_64"
	case "arm64":
		architecture = "aarch64"
	}
	return "demi-codex-provider/" + version.Release + " (" + osName + "; " + architecture + ")"
}

// Provider is a codex entry's provider for one account, shared by its requests.
type Provider struct {
	config       Config
	pool         provider.CredentialPool
	http         *http.Client
	clock        core.Clock
	responsesURL string
	modelsURL    string
	usageURL     string
	authURL      *url.URL
	quota        *provider.Quota
	accounts     *provider.Accounts
	// mu protects the time until which failed WebSocket connections are bypassed.
	mu               sync.Mutex
	unreachableUntil core.Timestamp
}

// New builds an account's provider over the entry's pool and quota snapshots.
func New(
	config Config,
	pool provider.CredentialPool,
	snapshots provider.QuotaSnapshotStore,
	client *http.Client,
	clock core.Clock,
) (*Provider, error) {
	if config.BackendURL == "" {
		config.BackendURL = "https://chatgpt.com/backend-api"
	}
	if config.AuthURL == "" {
		config.AuthURL = "https://auth.openai.com"
	}
	if config.HeaderTimeout == 0 {
		config.HeaderTimeout = 20 * time.Second
	}
	if config.ConnectTimeout == 0 {
		config.ConnectTimeout = 10 * time.Second
	}
	backend, err := url.Parse(config.BackendURL)
	if err != nil {
		return nil, fmt.Errorf("codex backend URL: %w", err)
	}
	auth, err := url.Parse(config.AuthURL)
	if err != nil {
		return nil, fmt.Errorf("codex sign-in URL: %w", err)
	}
	if config.Account != nil {
		account := *config.Account
		config.Account = &account
	}
	models := codexURL(backend, "/models")
	query := models.Query()
	query.Add("client_version", "0.153.4")
	models.RawQuery = query.Encode()
	p := &Provider{
		config:       config,
		pool:         pool,
		http:         client,
		clock:        clock,
		responsesURL: codexURL(backend, "/responses").String(),
		modelsURL:    models.String(),
		usageURL:     provider.EndpointURL(backend, "/wham/usage").String(),
		authURL:      auth,
	}
	p.quota = provider.NewQuota(&quotaSource{p: p}, snapshots, clock)
	p.accounts = provider.NewAccounts(pool, &loginKit{p: p}, clock)
	return p, nil
}

func codexURL(base *url.URL, path string) *url.URL {
	trimmed := strings.TrimRight(base.Path, "/")
	if strings.HasSuffix(trimmed, path) {
		return provider.EndpointURL(base, path)
	}
	if !strings.HasSuffix(trimmed, "/codex") {
		base = provider.EndpointURL(base, "/codex")
	}
	return provider.EndpointURL(base, path)
}

// Capabilities reports that Codex does not require a process Host.
func (*Provider) Capabilities() provider.Capabilities { return provider.Capabilities{} }

// AuthStatus reads the stored sign-in without refreshing it.
func (p *Provider) AuthStatus(ctx context.Context) core.AuthState {
	s, err := p.stored(ctx)
	if err != nil {
		return provider.AccountAuthFailure("Codex", err).State()
	}
	label := s.label().Label
	return &core.Authenticated{AccountLabel: &label}
}

// RuntimeState reports the backend used by this provider.
func (*Provider) RuntimeState() core.RuntimeState {
	message := "Uses the Codex backend with the account's ChatGPT sign-in"
	return &core.RuntimeReady{Message: &message}
}

// ReadFailure reads usage reset fields and standard retry headers.
func (*Provider) ReadFailure(d *core.ProviderErrorDiagnostics, now core.Timestamp) core.ProviderFailureFacts {
	return readFailure(d, now)
}

// Quota returns the account's quota source and snapshots.
func (p *Provider) Quota() *provider.Quota { return p.quota }

// Accounts returns the entry's shared account operations.
func (p *Provider) Accounts() provider.SubscriptionAccounts { return p.accounts }

// Runtime builds an independent session runtime.
func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.Runtime, error) {
	return &session{p: p, http: env.HTTP}, nil
}

type session struct {
	p    *Provider
	http *http.Client
}

// Fresh returns an independent runtime for another session.
func (s *session) Fresh() provider.Runtime { return &session{p: s.p, http: s.http} }

// Close releases the runtime resources.
func (*session) Close(context.Context) error { return nil }

// RequestLimits returns the request limits for the model.
func (*session) RequestLimits(core.Model) provider.RequestLimits {
	return provider.OpenAIRequestLimits()
}

var _ provider.Provider = (*Provider)(nil)

// authEndpoint appends a device-login or refresh path without changing the base query.
func (p *Provider) authEndpoint(path string) string {
	return provider.EndpointURL(p.authURL, path).String()
}
