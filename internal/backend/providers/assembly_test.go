package providers

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/providers/openaiapi"
	"github.com/wspl/demi/internal/webapi"
)

type testFamily struct {
	kind  webapi.CredentialKind
	build func(FamilyArgs) (provider.Provider, error)
}

func (f testFamily) Credential() webapi.CredentialKind                   { return f.kind }
func (testFamily) Wires() []core.WireAPI                                 { return nil }
func (f testFamily) Provider(args FamilyArgs) (provider.Provider, error) { return f.build(args) }

type accountProvider struct {
	*openaiapi.Provider
	accounts provider.SubscriptionAccounts
}

func (p *accountProvider) Accounts() provider.SubscriptionAccounts { return p.accounts }

type loginKit struct {
	started chan struct{}
	finish  chan struct{}
}

func (*loginKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true, Add: true}
}

func (k *loginKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	pending(core.LoginPending{VerificationURL: "https://example.test/login"})
	if k.started != nil {
		k.started <- struct{}{}
	}
	select {
	case <-ctx.Done():
		return provider.NewAccount{}, ctx.Err()
	case <-k.finish:
	}
	return provider.NewAccount{
		Secret: `{"token":"private-login-token"}`,
		Label:  provider.AccountLabel{Label: "Signed in"},
	}, nil
}

func (*loginKit) Add(input provider.AddAccount) (provider.NewAccount, error) {
	if strings.HasPrefix(input.SetupToken.Expose(), "bad-") {
		return provider.NewAccount{}, errors.New(input.SetupToken.Expose())
	}
	return provider.NewAccount{
		Secret: `{"token":"private-setup-token"}`,
		Label:  provider.AccountLabel{Label: input.SetupToken.Expose()[0:1]},
	}, nil
}

func assemblyFixture(t *testing.T, vault *Vault) *Assembly {
	t.Helper()
	clock := core.SystemClock{}
	a := NewAssembly(
		vault,
		&FamilyRegistry{},
		NewAccountQuotas(vault),
		NewModelCatalogCache(vault.control, clock),
		NewVendorCatalog(provider.NewModelsDevClient(http.DefaultClient, "http://unused.invalid", clock)),
		http.DefaultClient,
		clock,
	)
	t.Cleanup(func() {
		if err := a.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return a
}

func registerLogin(a *Assembly, name string, kit *loginKit) {
	a.families.Register(
		name,
		testFamily{kind: webapi.CredentialKindSubscription, build: func(
			args FamilyArgs,
		) (provider.Provider, error) {
			c, ok := args.Credential.(*SubscriptionArgs)
			if !ok {
				return nil, &FamilyError{Kind: FamilyWrongCredential}
			}
			return &accountProvider{
				Provider: openaiapi.New(openaiapi.Config{APIKey: "test"}, args.Clock),
				accounts: provider.NewAccounts(c.Pool, kit, args.Clock),
			}, nil
		}},
	)
}

func TestAssemblyRebuildsFreshConfigurationAndRedactsDetails(t *testing.T) {
	vault, owner := vaultFixture(t)
	a := assemblyFixture(t, vault)
	ctx := t.Context()
	var keys []string
	a.families.Register(
		"openai",
		testFamily{kind: webapi.CredentialKindAPIKey, build: func(args FamilyArgs) (provider.Provider, error) {
			c := args.Credential.(*APIKeyArgs)
			keys = append(keys, c.APIKey.Expose())
			return openaiapi.New(openaiapi.Config{APIKey: c.APIKey}, args.Clock), nil
		}},
	)
	entry, err := vault.CreateAPIKey(ctx, owner.ID, "openai", "Work", APIKeyConfig{APIKey: "private-first"})
	if err != nil {
		t.Fatal(err)
	}
	first, err := a.ProviderFor(ctx, entry)
	if err != nil {
		t.Fatal(err)
	}
	reused, err := a.ProviderFor(ctx, entry)
	if err != nil || first != reused {
		t.Fatal("unchanged entry rebuilt", err)
	}
	changed, err := vault.Update(ctx, entry.ID, nil, &APIKeyConfig{APIKey: "private-second"})
	if err != nil {
		t.Fatal(err)
	}
	second, err := a.ProviderFor(ctx, *changed)
	if err != nil || second == first {
		t.Fatal("changed entry reused", err)
	}
	// A late read of the old entry cannot pin subsequent fresh reads to it.
	if _, err := a.ProviderFor(ctx, entry); err != nil {
		t.Fatal(err)
	}
	if _, err := a.ProviderFor(ctx, *changed); err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(
		[]string{
			"private-first",
			"private-second",
			"private-first",
			"private-second",
		},
		keys,
	); diff != "" {
		t.Fatal(diff)
	}
	details, err := a.Details(ctx, *changed, false)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := contract.EncodeJSON(details)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), "private-") {
		t.Fatal("secret in provider details")
	}
	catalogs := a.ModelCatalog(
		ctx,
		[]ProviderEntry{
			*changed,
			{
				ID:         "missing",
				Family:     "unknown",
				Credential: &APIKeyConfig{APIKey: "test"},
			},
		},
		false,
	)
	if len(catalogs) != 2 || len(catalogs[0].Models) == 0 || len(catalogs[1].Warnings) == 0 {
		t.Fatal(catalogs)
	}
	if _, err := a.ProcessRuntime(ctx, *changed, nil, nil); err == nil {
		t.Fatal("HTTP family accepted process runtime")
	}
	registerLogin(a, SetupTokenFamily, &loginKit{})
	subscription, err := ImportSetupToken(ctx, a, owner.ID, "Subscription", "good-secret")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := AddToken(
		ctx,
		a,
		subscription,
		"bad-private-value",
	); err == nil ||
		strings.Contains(err.Error(), "bad-private-value") {
		t.Fatal("token import leaked", err)
	}
	secondAccount, err := AddToken(ctx, a, subscription, "next-secret")
	if err != nil {
		t.Fatal(err)
	}
	accounts, err := ListAccounts(ctx, a, subscription, true)
	if err != nil || len(accounts.Accounts) != 2 {
		t.Fatal(accounts, err)
	}
	concealed, err := ListAccounts(ctx, a, subscription, false)
	if err != nil || len(concealed.Accounts) != 0 || concealed.Active != nil {
		t.Fatal(concealed, err)
	}
	if err := RemoveAccount(ctx, a, subscription, *subscription.Active()); err == nil {
		t.Fatal("removed active account")
	}
	if _, err := ActivateAccount(ctx, a, subscription, webapi.CredentialID(secondAccount.ID)); err != nil {
		t.Fatal(err)
	}
	fresh, err := vault.Entry(ctx, subscription.ID)
	if err != nil {
		t.Fatal(err)
	}
	if err := RemoveAccount(ctx, a, *fresh, *subscription.Active()); err != nil {
		t.Fatal(err)
	}
	if _, err := a.ForAccount(ctx, *fresh, "absent"); err != nil {
		t.Fatal(err)
	}
	if err := a.Forget(ctx, fresh.ID); err != nil {
		t.Fatal(err)
	}
}

func TestLoginPublishesAtomicallyAndCancelReleasesEntry(t *testing.T) {
	vault, owner := vaultFixture(t)
	a := assemblyFixture(t, vault)
	kit := &loginKit{
		started: make(chan struct{}, 1),
		finish:  make(chan struct{}),
	}
	registerLogin(a, "device", kit)
	operations := &Operations{}
	flows := NewLoginFlows(a, operations, DefaultLoginTiming())
	defer func() {
		if err := flows.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	id, err := flows.Start(t.Context(), owner.ID, owner.ID, "device", "Device", nil)
	if err != nil {
		t.Fatal(err)
	}
	<-kit.started
	if flows.State(id, "foreign") != nil || flows.Cancel(t.Context(), id, "foreign") {
		t.Fatal("foreign login disclosed")
	}
	if _, ok := flows.State(id, owner.ID).(*webapi.LoginStatePending); !ok {
		t.Fatal("not pending")
	}
	entries, err := vault.Entries(t.Context(), owner.ID)
	if err != nil || len(entries) != 0 {
		t.Fatal("entry published before login", err)
	}
	flows.mu.Lock()
	done := flows.flows[id].done
	flows.mu.Unlock()
	close(kit.finish)
	<-done
	completed, ok := flows.State(id, owner.ID).(*webapi.LoginStateCompleted)
	if !ok {
		t.Fatal(flows.State(id, owner.ID))
	}
	entry, err := vault.Entry(t.Context(), completed.ProviderID)
	if err != nil || entry == nil || entry.Active() == nil {
		t.Fatal(entry, err)
	}
	if _, err := flows.Start(t.Context(), owner.ID, owner.ID, "device", "Duplicate", nil); err == nil {
		t.Fatal("duplicate subscription")
	}
	kit = &loginKit{
		started: make(chan struct{}, 1),
		finish:  make(chan struct{}),
	}
	registerLogin(a, "device", kit)
	if err := a.Invalidate(t.Context(), entry.ID); err != nil {
		t.Fatal(err)
	}
	id, err = flows.Start(t.Context(), owner.ID, owner.ID, "device", "Device", entry)
	if err != nil {
		t.Fatal(err)
	}
	<-kit.started
	if guard := operations.Reserve(entry.ID); guard != nil {
		guard.Release()
		t.Fatal("running login did not reserve entry")
	}
	if !flows.Cancel(t.Context(), id, owner.ID) {
		t.Fatal("cancel did not find login")
	}
	failed, ok := flows.State(id, owner.ID).(*webapi.LoginStateFailed)
	if !ok || failed.Message != "The login was cancelled" {
		t.Fatal(flows.State(id, owner.ID))
	}
	guard := operations.Reserve(entry.ID)
	if guard == nil {
		t.Fatal("reservation leaked")
	}
	guard.Release()
	guard.Release()
}

func TestLoginExpiresAndResultRetentionEnds(t *testing.T) {
	// The existing, accountless entry starts without storage IO; the timer and
	// login task can therefore run entirely in virtual time.
	synctest.Test(t, func(t *testing.T) {
		clock := core.SystemClock{}
		a := NewAssembly(nil, &FamilyRegistry{}, nil, nil, NewVendorCatalog(nil), http.DefaultClient, clock)
		registerLogin(a, "device", &loginKit{finish: make(chan struct{})})
		flows := NewLoginFlows(a, &Operations{}, LoginTiming{Lifetime: time.Minute, Retention: time.Minute})
		defer func() {
			if err := flows.Close(context.Background()); err != nil {
				t.Error(err)
			}
		}()
		entry := ProviderEntry{
			ID:         "entry",
			Family:     "device",
			Credential: &SubscriptionCredential{},
		}
		id, err := flows.Start(t.Context(), "owner", "starter", "device", "Device", &entry)
		if err != nil {
			t.Fatal(err)
		}
		flows.mu.Lock()
		done := flows.flows[id].done
		flows.mu.Unlock()
		before := time.Now()
		<-done
		state, ok := flows.State(id, "starter").(*webapi.LoginStateFailed)
		if !ok || state.Message != "The login expired" || time.Since(before) != time.Minute {
			t.Fatal(state, time.Since(before))
		}
		time.Sleep(time.Minute)
		if flows.State(id, "starter") != nil {
			t.Fatal("retention kept ended login")
		}
	})
}
