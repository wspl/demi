package backendtest

import (
	"context"
	"errors"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// AccountDirectory scripts successive provider catalog reads.
type AccountDirectory struct {
	mu      sync.Mutex
	answers []directoryAnswer
	reads   int
}
type directoryAnswer struct {
	catalog types.ProviderModelList
	err     error
}

// Answer queues the next directory response.
func (d *AccountDirectory) Answer(catalog types.ProviderModelList, err error) {
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

func (d *AccountDirectory) read() (types.ProviderModelList, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.reads++
	if len(d.answers) == 0 {
		return types.ProviderModelList{}, errors.New("the directory has no answer scripted")
	}
	a := d.answers[0]
	d.answers = d.answers[1:]
	return a.catalog, a.err
}

// AccountCatalog makes the models used by directory cache scenarios.
func AccountCatalog(names ...string) types.ProviderModelList {
	result := types.ProviderModelList{
		Models:          []types.ProviderModel{},
		Warnings:        []string{},
		SourceFetchedAt: "2026-09-24T07:00:00.000Z",
	}
	contextWindow, output := uint32(100000), uint32(8000)
	yes := true
	low := "low"
	efforts := []string{"low", "high"}
	for _, name := range names {
		result.Models = append(
			result.Models,
			types.ProviderModel{
				ID:                       name,
				DisplayName:              name + " model",
				ContextWindow:            &contextWindow,
				OutputLimit:              &output,
				SupportsTools:            &yes,
				SupportsAttachments:      &yes,
				SupportsReasoning:        &yes,
				SupportedThinkingEfforts: &efforts,
				DefaultThinkingEffort:    &low,
				ServiceTiers:             []types.ServiceTier{},
			},
		)
	}
	return result
}

// AccountLoginScript holds device logins until the scenario approves them.
type AccountLoginScript struct {
	mu       sync.Mutex
	approved bool
	changed  chan struct{}
	// Cancelled counts logins whose wait ended through cancellation.
	Cancelled atomic.Int64
}

// SetApproved changes whether pending and future logins can complete.
func (s *AccountLoginScript) SetApproved(value bool) {
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
	// T owns assertions and cleanup for the scripted family.
	T testing.TB
	// Directory supplies scripted provider catalog responses.
	Directory *AccountDirectory
	// Login supplies approval and cancellation observations for device logins.
	Login *AccountLoginScript
	// Cost sets the scripted quota probe cost.
	Cost *provider.ProbeCost
	// WireTypes lists selectable wire protocols.
	WireTypes []types.WireAPI
}

// AccountFamilies registers the two subscription families used by account scenarios.
func AccountFamilies(t testing.TB, cost *provider.ProbeCost) (*providerhost.FamilyRegistry, *AccountLoginScript) {
	login := &AccountLoginScript{}
	registry := backend.BuiltinFamilies()
	for _, name := range []string{"claude-code", "device"} {
		registry.Register(name, &AccountFamily{T: t, Directory: &AccountDirectory{}, Login: login, Cost: cost})
	}
	return registry, login
}

// Credential identifies which entry credentials this scripted family accepts.
func (f *AccountFamily) Credential() webapiproto.CredentialKind {
	if f.Login != nil {
		return webapiproto.CredentialKindSubscription
	}
	return webapiproto.CredentialKindAPIKey
}

// Wires lists the protocols selectable for this scripted family.
func (f *AccountFamily) Wires() []types.WireAPI { return f.WireTypes }

// Provider binds the script to the entry and its selected account.
func (f *AccountFamily) Provider(args providerhost.FamilyArgs) (provider.Provider, error) {
	p := &accountProvider{family: f}
	if subscription, ok := args.Credential.(*providerhost.SubscriptionArgs); ok {
		p.accounts = provider.NewAccounts(subscription.Pool, &accountKit{script: f.Login}, args.Clock)
		if subscription.Account != nil {
			p.account = &subscription.Account.CredentialID
			p.quota = provider.NewQuota(accountQuota{cost: f.Cost}, subscription.Account.Quota, args.Clock)
		}
	}
	return p, nil
}

type accountKit struct{ script *AccountLoginScript }

// Capability declares the scripted login and account addition capabilities.
func (k *accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true, Add: true}
}

// Login waits for approval and returns the scripted device account.
func (k *accountKit) Login(ctx context.Context, pending func(types.LoginPending)) (provider.NewAccount, error) {
	code := "ABCD-1234"
	pending(types.LoginPending{VerificationURL: "https://verify.example/device", UserCode: &code})
	if err := k.script.wait(ctx); err != nil {
		return provider.NewAccount{}, err
	}
	identity := "device"
	return provider.NewAccount{
		Secret: `{"token":"login-secret"}`,
		Label:  provider.AccountLabel{Label: "device@example.test", IdentityKey: &identity},
	}, nil
}

// Add validates the scripted setup token and returns its account.
func (k *accountKit) Add(input provider.AddAccount) (provider.NewAccount, error) {
	token := input.SetupToken.Expose()
	if strings.HasPrefix(token, "bad") {
		return provider.NewAccount{}, errors.New(token + " is not a setup token")
	}
	secret, err := contract.EncodeObject([]contract.Field{{Name: "setupToken", Value: token}})
	if err != nil {
		return provider.NewAccount{}, err
	}
	return provider.NewAccount{
		Secret: string(secret),
		Label:  provider.AccountLabel{Label: "Account " + token[len(token)-1:], IdentityKey: &token},
	}, nil
}

type accountQuota struct{ cost *provider.ProbeCost }

// ProbeCost returns the configured quota probe cost.
func (q accountQuota) ProbeCost() (provider.ProbeCost, bool) {
	if q.cost == nil {
		return 0, false
	}
	return *q.cost, true
}

// Probe returns the scripted quota reading.
func (q accountQuota) Probe(context.Context) (provider.ProbeReading, error) {
	label := "device@example.test"
	used := float64(40)
	return provider.ProbeReading{
		AccountLabel: &label,
		Windows:      []types.QuotaWindow{{ID: "weekly", Label: "Weekly", UsedPercent: &used}},
	}, nil
}

// Observe reports no quota windows from fixture observations.
func (accountQuota) Observe(provider.Observation) []types.QuotaWindow { return nil }

type accountProvider struct {
	family   *AccountFamily
	accounts provider.SubscriptionAccounts
	quota    *provider.Quota
	account  *string
}

// Capabilities returns the fixture provider capabilities.
func (p *accountProvider) Capabilities() provider.Capabilities { return provider.Capabilities{} }

// AuthStatus reports the fixture authentication state.
func (p *accountProvider) AuthStatus(context.Context) types.AuthState {
	return &types.Authenticated{AccountLabel: p.account}
}

// RuntimeState reports that the fixture runtime is ready.
func (p *accountProvider) RuntimeState() types.RuntimeState { return &types.RuntimeReady{} }

// ListModels returns the scripted provider catalog.
func (p *accountProvider) ListModels(context.Context) (types.ProviderModelList, error) {
	return p.family.Directory.read()
}

// ReadFailure returns empty failure facts for the fixture provider.
func (p *accountProvider) ReadFailure(*types.ProviderErrorDiagnostics, types.Timestamp) types.ProviderFailureFacts {
	return types.ProviderFailureFacts{}
}

// Quota returns the fixture quota capability.
func (p *accountProvider) Quota() *provider.Quota { return p.quota }

// Accounts returns the fixture subscription accounts capability.
func (p *accountProvider) Accounts() provider.SubscriptionAccounts { return p.accounts }

// Runtime creates the scripted fixture runtime.
func (p *accountProvider) Runtime(provider.RuntimeEnv) (provider.Runtime, error) {
	return providertest.NewScriptedRuntime(
		p.family.T,
		providertest.Events(providertest.Text("ok"), providertest.Response(1, 1)),
	), nil
}
