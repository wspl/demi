package provider_test

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

type tokens = providertest.TokenDocument

func accountMeta(id string) provider.AccountMeta {
	return provider.AccountMeta{ID: id, Label: id + "@example.com", UpdatedAt: now, Source: "test", IdentityKey: &id}
}
func poolWith(t *testing.T, id string, value tokens) *provider.MemoryCredentialPool {
	t.Helper()
	pool := provider.NewMemoryCredentialPool()
	if err := pool.Write(t.Context(), accountMeta(id), encoded(t, value)); err != nil {
		t.Fatal(err)
	}
	return pool
}
func storedTokens(t *testing.T, pool *provider.MemoryCredentialPool, id string) tokens {
	t.Helper()
	value, err := provider.ReadSecret(t.Context(), pool.Document(id), providertest.DecodeTokenDocument)
	if err != nil {
		t.Fatal(err)
	}
	return value.Secret
}
func TestRefreshTurnsFIFO(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var gate provider.RefreshGates
		first, err := gate.Turn(t.Context(), "a")
		if err != nil {
			t.Fatal(err)
		}
		defer first.Release()
		other, err := gate.Turn(t.Context(), "b")
		if err != nil {
			t.Fatal(err)
		}
		defer other.Release()
		second := make(chan *gates.Permit, 1)
		abandoned := make(chan error, 1)
		third := make(chan *gates.Permit, 1)
		var workers sync.WaitGroup
		workers.Go(func() {
			p, e := gate.Turn(t.Context(), "a")
			if e != nil {
				t.Error(e)
			}
			second <- p
		})
		synctest.Wait()
		ctx, cancel := context.WithCancel(t.Context())
		workers.Go(func() {
			p, e := gate.Turn(ctx, "a")
			if p != nil {
				p.Release()
			}
			abandoned <- e
		})
		synctest.Wait()
		cancel()
		if err := <-abandoned; !errors.Is(err, context.Canceled) {
			t.Fatalf("%v", err)
		}
		workers.Go(func() {
			p, e := gate.Turn(t.Context(), "a")
			if e != nil {
				t.Error(e)
			}
			third <- p
		})
		synctest.Wait()
		first.Release()
		held := <-second
		defer held.Release()
		synctest.Wait()
		select {
		case p := <-third:
			p.Release()
			t.Fatal("third ran beside second")
		default:
		}
		held.Release()
		(<-third).Release()
		workers.Wait()
	})
}
func TestDueSecretRefreshAndStore(t *testing.T) {
	pool := poolWith(t, "a", tokens{Access: "old", Refresh: "r1"})
	before, err := pool.Document("a").Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	got, err := provider.Renew(t.Context(), pool.Document("a"), providertest.DecodeTokenDocument, func(tokens) bool { return true }, func(_ context.Context, value tokens) (tokens, error) {
		requireEqual(t, value.Refresh, "r1")
		return tokens{Access: "new", Refresh: "r2"}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, got, tokens{Access: "new", Refresh: "r2"})
	after, err := pool.Document("a").Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, after.Text, encoded(t, got))
	if after.Version == before.Version {
		t.Fatal("revision did not advance")
	}
}
func TestWaitingRefresherRereadsAndRechecks(t *testing.T) {
	for _, always := range []bool{false, true} {
		synctest.Test(t, func(t *testing.T) {
			pool := poolWith(t, "a", tokens{Access: "old", Refresh: "r1"})
			entered := make(chan struct{}, 2)
			release := make(chan struct{})
			var spent []string
			refresh := func(ctx context.Context, value tokens) (tokens, error) {
				spent = append(spent, value.Refresh)
				entered <- struct{}{}
				select {
				case <-release:
				case <-ctx.Done():
					return tokens{}, ctx.Err()
				}
				return tokens{Access: "new", Refresh: "after-" + value.Refresh}, nil
			}
			due := func(value tokens) bool { return always || value.Access == "old" }
			results := make(chan tokens, 2)
			var workers sync.WaitGroup
			for range 2 {
				workers.Go(func() {
					value, err := provider.Renew(t.Context(), pool.Document("a"), providertest.DecodeTokenDocument, due, refresh)
					if err != nil {
						t.Error(err)
					}
					results <- value
				})
			}
			<-entered
			synctest.Wait()
			close(release)
			first, second := <-results, <-results
			workers.Wait()
			requireEqual(t, first.Access, "new")
			requireEqual(t, second.Access, "new")
			want := []string{"r1"}
			if always {
				want = append(want, "after-r1")
				requireEqual(t, storedTokens(t, pool, "a"), tokens{Access: "new", Refresh: "after-after-r1"})
			}
			requireEqual(t, spent, want)
		})
	}
}
func TestRefusedRefreshUsesConcurrentWriter(t *testing.T) {
	pool := poolWith(t, "a", tokens{Access: "old", Refresh: "r1"})
	refused := errors.New("refresh token revoked")
	got, err := provider.Renew(t.Context(), pool.Document("a"), providertest.DecodeTokenDocument, func(tokens) bool { return true }, func(ctx context.Context, _ tokens) (tokens, error) {
		if err := pool.Write(ctx, accountMeta("a"), encoded(t, tokens{Access: "winner", Refresh: "rw"})); err != nil {
			return tokens{}, err
		}
		return tokens{}, refused
	})
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, got, tokens{Access: "winner", Refresh: "rw"})
	_, err = provider.Renew(t.Context(), pool.Document("a"), providertest.DecodeTokenDocument, func(tokens) bool { return true }, func(context.Context, tokens) (tokens, error) { return tokens{}, refused })
	var renewal *provider.RenewError
	if !errors.Is(err, refused) || !errors.As(err, &renewal) {
		t.Fatalf("%v", err)
	}
}
func TestRefreshReplaceLosesToWriter(t *testing.T) {
	pool := poolWith(t, "a", tokens{Access: "old", Refresh: "r1"})
	got, err := provider.Renew(t.Context(), pool.Document("a"), providertest.DecodeTokenDocument, func(tokens) bool { return true }, func(ctx context.Context, _ tokens) (tokens, error) {
		if err := pool.Write(ctx, accountMeta("a"), encoded(t, tokens{Access: "winner", Refresh: "rw"})); err != nil {
			return tokens{}, err
		}
		return tokens{Access: "loser", Refresh: "rl"}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, got, tokens{Access: "winner", Refresh: "rw"})
	requireEqual(t, storedTokens(t, pool, "a"), got)
}
func TestCorruptSecretPathNeverQuotesValue(t *testing.T) {
	pool := provider.NewMemoryCredentialPool()
	for _, tc := range []struct {
		text, path string
		fault      provider.SecretFault
	}{
		{`{"access":"sk-secret-1","refresh":7}`, "refresh", provider.SecretShape},
		{`{"access":"sk-secret-1"}`, ".", provider.SecretShape},
		{`{"access":"sk-secret-1","refresh":"r","extra":"sk-secret-2"}`, "extra", provider.SecretShape},
		{`{"access":"sk-secret-1"`, ".", provider.SecretSyntax},
		{`{"access":"sk-secret-1","refresh":"r"} trailing`, ".", provider.SecretSyntax},
	} {
		if err := pool.Write(t.Context(), accountMeta("a"), tc.text); err != nil {
			t.Fatal(err)
		}
		_, err := provider.ReadSecret(t.Context(), pool.Document("a"), providertest.DecodeTokenDocument)
		var account *provider.AccountError
		var decode *provider.SecretDecodeError
		if !errors.As(err, &account) || account.Kind != provider.AccountInvalid || !errors.As(err, &decode) {
			t.Fatalf("%v", err)
		}
		requireEqual(t, decode.Path, tc.path)
		requireEqual(t, decode.Fault, tc.fault)
		if strings.Contains(err.Error(), "sk-secret") {
			t.Fatal("secret leaked")
		}
	}
	_, err := provider.ReadSecret(t.Context(), pool.Document("absent"), providertest.DecodeTokenDocument)
	var account *provider.AccountError
	if !errors.As(err, &account) || account.Kind != provider.AccountMissing {
		t.Fatalf("%v", err)
	}
}
func TestMemoryPoolVersionsAndActive(t *testing.T) {
	pool := provider.NewMemoryCredentialPool()
	for _, id := range []string{"b", "a"} {
		if err := pool.Write(t.Context(), accountMeta(id), "{}"); err != nil {
			t.Fatal(err)
		}
	}
	listed, err := pool.List(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, []string{listed[0].ID, listed[1].ID}, []string{"a", "b"})
	doc := pool.Document("a")
	first, err := doc.Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	second, err := doc.Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if ok, err := doc.Replace(t.Context(), "two", first.Version); err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	if ok, err := doc.Replace(t.Context(), "lost", second.Version); err != nil || ok {
		t.Fatalf("%v %v", ok, err)
	}
	latest, err := doc.Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, latest.Text, "two")
	if missing, err := pool.Document("absent").Read(t.Context()); err != nil || missing != nil {
		t.Fatalf("%v %v", missing, err)
	}
	if active, err := pool.Active(t.Context()); err != nil || active != nil {
		t.Fatalf("%v %v", active, err)
	}
	if err := pool.SetActive(t.Context(), "zz"); err == nil {
		t.Fatal("selected missing account")
	}
	if err := pool.SetActive(t.Context(), "b"); err != nil {
		t.Fatal(err)
	}
	active, err := pool.Active(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, *active, "b")
	if err := pool.Remove(t.Context(), "b"); err != nil {
		t.Fatal(err)
	}
	if active, err := pool.Active(t.Context()); err != nil || active != nil {
		t.Fatalf("%v %v", active, err)
	}
	entries := pool.Entries()
	if len(entries) != 1 || entries[0].Meta.ID != "a" || entries[0].Secret != "two" {
		t.Fatalf("%+v", entries)
	}
}
func TestCredentialID(t *testing.T) {
	key := "acct-1"
	requireEqual(t, provider.CredentialIDFor(&key, "a@example.com"), "cred-ba36a4edd92d37c6")
	requireEqual(t, provider.CredentialIDFor(nil, "label only"), "cred-db98004c5bc389e4")
	key = ""
	requireEqual(t, provider.CredentialIDFor(&key, "label only"), "cred-db98004c5bc389e4")
}

type accountKit struct{ logins []provider.NewAccount }

func (*accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true}
}
func (k *accountKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	code := "ABCD-1234"
	pending(core.LoginPending{VerificationURL: "https://vendor.example/device", UserCode: &code})
	if len(k.logins) == 0 {
		<-ctx.Done()
		return provider.NewAccount{}, ctx.Err()
	}
	next := k.logins[0]
	k.logins = k.logins[1:]
	return next, nil
}
func (*accountKit) Add(provider.AddAccount) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrAccountsUnsupported
}
func newAccount(identity, secret string) provider.NewAccount {
	detail := "device"
	return provider.NewAccount{Secret: secret, Label: provider.AccountLabel{Label: identity + "@example.com", Detail: &detail, IdentityKey: &identity}}
}
func TestLoginImportsIdentityAndSelectsFirst(t *testing.T) {
	pool := provider.NewMemoryCredentialPool()
	kit := &accountKit{logins: []provider.NewAccount{newAccount("acct-1", "first secret"), newAccount("acct-1", "second secret"), newAccount("acct-2", "other secret")}}
	accounts := provider.NewAccounts(pool, kit, providertest.FixedClock(now))
	requireEqual(t, accounts.Capability(), provider.AccountsCapability{Login: true})
	shown := []string{}
	report := func(p core.LoginPending) { shown = append(shown, *p.UserCode) }
	first, err := accounts.Login(t.Context(), report)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, first.ID, "cred-ba36a4edd92d37c6")
	requireEqual(t, first.Label, "acct-1@example.com")
	requireEqual(t, *first.UpdatedAt, now)
	requireEqual(t, shown, []string{"ABCD-1234"})
	active, err := accounts.Active(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, *active, first.ID)
	again, err := accounts.Login(t.Context(), report)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, again.ID, first.ID)
	revision, err := pool.Document(first.ID).Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, revision.Text, "second secret")
	other, err := accounts.Login(t.Context(), report)
	if err != nil {
		t.Fatal(err)
	}
	if other.ID == first.ID {
		t.Fatal("identities collided")
	}
	listed, err := accounts.List(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, len(listed), 2)
	active, err = accounts.Active(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, *active, first.ID)
	meta, err := pool.Meta(t.Context(), other.ID)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, meta.Source, "login:device")
	if err := accounts.Remove(t.Context(), first.ID); !errors.Is(err, provider.ErrActiveAccount) {
		t.Fatalf("%v", err)
	}
	if err := accounts.Remove(t.Context(), other.ID); err != nil {
		t.Fatal(err)
	}
	for _, err := range []error{accounts.Remove(t.Context(), "cred-absent"), accounts.SetActive(t.Context(), "cred-absent")} {
		var missing *provider.PoolError
		if !errors.As(err, &missing) || missing.ID != "cred-absent" {
			t.Fatalf("%v", err)
		}
	}
	listed, err = accounts.List(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if len(listed) != 1 || listed[0].ID != first.ID {
		t.Fatalf("%+v", listed)
	}
	secret, err := provider.NewSecret("sk-ant-oat01-x")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := accounts.Add(t.Context(), provider.AddAccount{SetupToken: secret}); !errors.Is(err, provider.ErrAccountsUnsupported) {
		t.Fatalf("%v", err)
	}
}
func TestCanceledLoginStoresNothing(t *testing.T) {
	pool := provider.NewMemoryCredentialPool()
	accounts := provider.NewAccounts(pool, &accountKit{}, providertest.FixedClock(now))
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	_, err := accounts.Login(ctx, func(core.LoginPending) { cancel() })
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("%v", err)
	}
	listed, err := pool.List(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, len(listed), 0)
}
