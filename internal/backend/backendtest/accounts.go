package backendtest

import (
	"context"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

// AccountDirectory scripts successive provider catalog reads.
type AccountDirectory struct {
	mu      sync.Mutex
	answers []directoryAnswer
	reads   int
}
type directoryAnswer struct {
	catalog core.ProviderModelList
	err     error
}

// Answer queues the next directory response.
func (d *AccountDirectory) Answer(catalog core.ProviderModelList, err error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.answers = append(d.answers, directoryAnswer{catalog, err})
}

// Reads counts requests that reached the family directory.
func (d *AccountDirectory) Reads() int {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.reads
}
func (d *AccountDirectory) read() (core.ProviderModelList, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.reads++
	if len(d.answers) == 0 {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: "the directory has no answer scripted"}
	}
	a := d.answers[0]
	d.answers = d.answers[1:]
	return a.catalog, a.err
}

// AccountCatalog makes the models used by directory cache scenarios.
func AccountCatalog(names ...string) core.ProviderModelList {
	result := core.ProviderModelList{Models: []core.ProviderModel{}, Warnings: []string{}, SourceFetchedAt: "2026-09-24T07:00:00.000Z"}
	contextWindow, output := uint32(100000), uint32(8000)
	yes := true
	low := "low"
	efforts := []string{"low", "high"}
	for _, name := range names {
		result.Models = append(result.Models, core.ProviderModel{ID: name, DisplayName: name + " model", ContextWindow: &contextWindow, OutputLimit: &output, SupportsTools: &yes, SupportsAttachments: &yes, SupportsReasoning: &yes, SupportedThinkingEfforts: &efforts, DefaultThinkingEffort: &low, ServiceTiers: []core.ServiceTier{}})
	}
	return result
}

// AccountLoginScript holds device logins until the scenario approves them.
type AccountLoginScript struct {
	mu        sync.Mutex
	approved  bool
	changed   chan struct{}
	Cancelled atomic.Int64
}

// Approve changes whether pending and future logins can complete.
func (s *AccountLoginScript) Approve(value bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.approved = value
	if s.changed != nil {
		close(s.changed)
	}
	s.changed = make(chan struct{})
}
func (s *AccountLoginScript) wait(ctx context.Context) error {
	for {
		s.mu.Lock()
		approved := s.approved
		if s.changed == nil {
			s.changed = make(chan struct{})
		}
		changed := s.changed
		s.mu.Unlock()
		if approved {
			return nil
		}
		select {
		case <-ctx.Done():
			s.Cancelled.Add(1)
			return ctx.Err()
		case <-changed:
		}
	}
}

// AccountFamily replaces a subscription or API-key family at the real assembly boundary.
type AccountFamily struct {
	T         testing.TB
	Directory *AccountDirectory
	Login     *AccountLoginScript
	Cost      *provider.ProbeCost
	WireTypes []core.WireAPI
}

// AccountFamilies registers the two subscription families used by account scenarios.
func AccountFamilies(t testing.TB, cost *provider.ProbeCost) (*providers.FamilyRegistry, *AccountLoginScript) {
	login := &AccountLoginScript{}
	registry := backend.BuiltinFamilies()
	for _, name := range []string{"claude-code", "device"} {
		registry.Register(name, &AccountFamily{T: t, Directory: &AccountDirectory{}, Login: login, Cost: cost})
	}
	return registry, login
}

// Credential identifies which entry credentials this scripted family accepts.
func (f *AccountFamily) Credential() webapi.CredentialKind {
	if f.Login != nil {
		return webapi.CredentialKindSubscription
	}
	return webapi.CredentialKindAPIKey
}

// Wires lists the protocols selectable for this scripted family.
func (f *AccountFamily) Wires() []core.WireAPI { return f.WireTypes }

// Provider binds the script to the entry and its selected account.
func (f *AccountFamily) Provider(args providers.FamilyArgs) (provider.Provider, error) {
	p := &accountProvider{family: f}
	if subscription, ok := args.Credential.(*providers.SubscriptionArgs); ok {
		p.accounts = provider.NewAccounts(subscription.Pool, &accountKit{script: f.Login}, args.Clock)
		if subscription.Account != nil {
			p.account = &subscription.Account.CredentialID
			p.quota = provider.NewQuota(accountQuota{cost: f.Cost}, subscription.Account.Quota, args.Clock)
		}
	}
	return p, nil
}

type accountKit struct{ script *AccountLoginScript }

func (k *accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true, Add: true}
}
func (k *accountKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	code := "ABCD-1234"
	pending(core.LoginPending{VerificationURL: "https://verify.example/device", UserCode: &code})
	if err := k.script.wait(ctx); err != nil {
		return provider.NewAccount{}, err
	}
	identity := "device"
	return provider.NewAccount{Secret: `{"token":"login-secret"}`, Label: provider.AccountLabel{Label: "device@example.test", IdentityKey: &identity}}, nil
}
func (k *accountKit) Add(input provider.AddAccount) (provider.NewAccount, error) {
	token := input.SetupToken.Expose()
	if strings.HasPrefix(token, "bad") {
		return provider.NewAccount{}, &provider.AccountsError{Message: token + " is not a setup token"}
	}
	secret, err := contract.EncodeObject([]contract.Field{{Name: "setupToken", Value: token}})
	if err != nil {
		return provider.NewAccount{}, err
	}
	return provider.NewAccount{Secret: string(secret), Label: provider.AccountLabel{Label: "Account " + token[len(token)-1:], IdentityKey: &token}}, nil
}

type accountQuota struct{ cost *provider.ProbeCost }

func (q accountQuota) ProbeCost() *provider.ProbeCost { return q.cost }
func (q accountQuota) Probe(context.Context) (provider.ProbeReading, error) {
	label := "device@example.test"
	used := float64(40)
	return provider.ProbeReading{AccountLabel: &label, Windows: []core.QuotaWindow{{ID: "weekly", Label: "Weekly", UsedPercent: &used}}}, nil
}
func (accountQuota) Observe(provider.Observation) []core.QuotaWindow { return nil }

type accountProvider struct {
	family   *AccountFamily
	accounts provider.SubscriptionAccounts
	quota    *provider.Quota
	account  *string
}

func (p *accountProvider) Capabilities() provider.Capabilities { return provider.Capabilities{} }
func (p *accountProvider) AuthStatus(context.Context) core.AuthState {
	return &core.Authenticated{AccountLabel: p.account}
}
func (p *accountProvider) RuntimeState() core.RuntimeState { return &core.RuntimeReady{} }
func (p *accountProvider) ListModels(context.Context) (core.ProviderModelList, error) {
	return p.family.Directory.read()
}
func (p *accountProvider) ReadFailure(*core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts {
	return core.ProviderFailureFacts{}
}
func (p *accountProvider) Quota() *provider.Quota                  { return p.quota }
func (p *accountProvider) Accounts() provider.SubscriptionAccounts { return p.accounts }
func (p *accountProvider) Runtime(provider.RuntimeEnv) (provider.Runtime, error) {
	return providertest.NewScriptedRuntime(p.family.T, providertest.Events(providertest.Text("ok"), providertest.Response(1, 1))), nil
}

// AccountSocket reads conversation frames for provider-account scenarios.
type AccountSocket struct{ Conn *websocket.Conn }

// OpenAccountSocket connects the authenticated product page and registers cleanup.
func OpenAccountSocket(ctx context.Context, t testing.TB, b *TestBackend, s *Session, id string) (*AccountSocket, error) {
	conn, response, err := websocket.Dial(ctx, b.WSURL("/api/conversations/"+id+"/stream"), &websocket.DialOptions{HTTPClient: b.HTTP, HTTPHeader: http.Header{"Cookie": []string{s.Cookie}, "Origin": []string{b.URL}}})
	if err != nil {
		if response != nil {
			return nil, fmt.Errorf("conversation upgrade HTTP %d: %w", response.StatusCode, err)
		}
		return nil, err
	}
	conn.SetReadLimit(-1)
	t.Cleanup(func() {
		// Closing an already failed or closed socket is normal teardown.
		_ = conn.CloseNow()
	})
	return &AccountSocket{Conn: conn}, nil
}

// Send writes a generated client frame.
func (s *AccountSocket) Send(ctx context.Context, frame framewire.ClientFrame) error {
	data, err := contract.EncodeJSON(frame)
	if err != nil {
		return err
	}
	return s.Conn.Write(ctx, websocket.MessageText, data)
}

// Next validates the next frame and uses a deadline only as a hang guard.
func (s *AccountSocket) Next(ctx context.Context) (framewire.ServerFrame, error) {
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	_, data, err := s.Conn.Read(ctx)
	if err != nil {
		return nil, err
	}
	return framewire.DecodeServerFrame(data)
}

// Open reads the handshake through pending steers.
func (s *AccountSocket) Open(ctx context.Context) error {
	if err := s.Send(ctx, &framewire.OpenFrame{}); err != nil {
		return err
	}
	first, err := s.Next(ctx)
	if err != nil {
		return err
	}
	if _, ok := first.(*framewire.OpenedFrame); !ok {
		return fmt.Errorf("conversation opening: %#v", first)
	}
	for {
		frame, err := s.Next(ctx)
		if err != nil {
			return err
		}
		if _, ok := frame.(*framewire.PendingSteersFrame); ok {
			return nil
		}
	}
}

// Chat sends a user message and reads through the running-to-idle transition.
func (s *AccountSocket) Chat(ctx context.Context, id, text string) ([]framewire.ServerFrame, error) {
	turn, err := core.ParseTurnID(id)
	if err != nil {
		return nil, err
	}
	if err := s.Send(ctx, &framewire.SendFrame{MessageID: turn, Content: []framewire.ClientContent{&framewire.TextContent{Text: text}}}); err != nil {
		return nil, err
	}
	return s.UntilIdle(ctx)
}

// UntilIdle reads frames through the idle phase following running.
func (s *AccountSocket) UntilIdle(ctx context.Context) ([]framewire.ServerFrame, error) {
	running := false
	var frames []framewire.ServerFrame
	for {
		frame, err := s.Next(ctx)
		if err != nil {
			return nil, err
		}
		frames = append(frames, frame)
		if phase, ok := frame.(*framewire.PhaseFrame); ok {
			if phase.Phase == core.SessionPhaseRunning {
				running = true
			}
			if phase.Phase == core.SessionPhaseIdle && running {
				return frames, nil
			}
		}
	}
}
