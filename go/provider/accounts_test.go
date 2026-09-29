package provider_test

import (
	"context"
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/providertest"
)

type accountKit struct{ account provider.NewAccount }

func (k *accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true, Add: true}
}
func (k *accountKit) Add(provider.AddAccount) (provider.NewAccount, error) { return k.account, nil }
func (k *accountKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	pending(core.LoginPending{VerificationURL: "https://vendor.invalid/device"})
	return k.account, ctx.Err()
}
func TestAccountsImportReplaceSelectAndRemove(t *testing.T) {
	ctx := t.Context()
	pool := provider.NewMemoryCredentialPool()
	identity := "same-person"
	kit := &accountKit{account: provider.NewAccount{Secret: "first", Label: provider.AccountLabel{Label: "Person", IdentityKey: &identity}}}
	accounts := provider.NewAccounts(pool, kit, providertest.FixedClock{})
	first, err := accounts.Add(ctx, provider.AddAccount{})
	if err != nil {
		t.Fatal(err)
	}
	active, err := accounts.Active(ctx)
	if err != nil || active == nil || *active != first.ID {
		t.Fatalf("active: %v %v", active, err)
	}
	if !errors.Is(accounts.Remove(ctx, first.ID), provider.ErrActiveAccount) {
		t.Fatal("removed active account")
	}
	kit.account.Secret = "replacement"
	kit.account.Label.Label = "Renamed"
	calls := 0
	again, err := accounts.Login(ctx, func(core.LoginPending) { calls++ })
	if err != nil || calls != 1 || again.ID != first.ID || again.Label != "Renamed" {
		t.Fatalf("replacement: %+v %v pending=%d", again, err, calls)
	}
	revision, err := pool.Document(first.ID).Read(ctx)
	if err != nil || revision.Text != "replacement" || revision.Version != 2 {
		t.Fatalf("document: %+v %v", revision, err)
	}
	kit.account.Label.IdentityKey = nil
	kit.account.Label.Label = "Other"
	other, err := accounts.Add(ctx, provider.AddAccount{})
	if err != nil {
		t.Fatal(err)
	}
	if err := accounts.SetActive(ctx, other.ID); err != nil {
		t.Fatal(err)
	}
	if err := accounts.Remove(ctx, first.ID); err != nil {
		t.Fatal(err)
	}
	listed, err := accounts.List(ctx)
	if err != nil || len(listed) != 1 || listed[0].ID != other.ID {
		t.Fatalf("list: %+v %v", listed, err)
	}
	var missing provider.PoolNotFound
	if !errors.As(accounts.Remove(ctx, first.ID), &missing) {
		t.Fatal("missing removal not refused")
	}
	cancelCtx, cancel := context.WithCancel(ctx)
	cancel()
	kit.account.Label.Label = "Cancelled"
	if _, err := accounts.Login(cancelCtx, func(core.LoginPending) {}); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancelled login: %v", err)
	}
	after, _ := accounts.List(ctx)
	if !reflect.DeepEqual(listed, after) {
		t.Fatal("cancelled login published")
	}
}
func TestCredentialPoolVersionsAndMetadataIsolation(t *testing.T) {
	ctx := t.Context()
	pool := provider.NewMemoryCredentialPool()
	doc := pool.Document("b")
	if revision, err := doc.Read(ctx); revision != nil || err != nil {
		t.Fatal(revision, err)
	}
	for _, id := range []string{"b", "a"} {
		detail := "original"
		if err := pool.Write(ctx, provider.AccountMeta{ID: id, Detail: &detail}, "secret"); err != nil {
			t.Fatal(err)
		}
		detail = "mutated"
	}
	metas, err := pool.List(ctx)
	if err != nil || metas[0].ID != "a" || *metas[0].Detail != "original" {
		t.Fatal(metas, err)
	}
	*metas[0].Detail = "mutated again"
	stored, _ := pool.Meta(ctx, "a")
	if *stored.Detail != "original" {
		t.Fatal("metadata aliases pool")
	}
	if active, _ := pool.Active(ctx); active != nil {
		t.Fatal("write selected account")
	}
	if err := pool.SetActive(ctx, "missing"); err == nil {
		t.Fatal("selected missing account")
	}
	revision, _ := doc.Read(ctx)
	if ok, err := doc.Replace(ctx, "winner", revision.Version); !ok || err != nil {
		t.Fatal(ok, err)
	}
	if ok, err := doc.Replace(ctx, "loser", revision.Version); ok || err != nil {
		t.Fatal(ok, err)
	}
	current, _ := doc.Read(ctx)
	if current.Text != "winner" || current.Version == revision.Version {
		t.Fatal(current)
	}
	if err := pool.SetActive(ctx, "b"); err != nil {
		t.Fatal(err)
	}
	if err := pool.Remove(ctx, "b"); err != nil {
		t.Fatal(err)
	}
	if active, _ := pool.Active(ctx); active != nil {
		t.Fatal("removed selection retained")
	}
	if ok, _ := doc.Replace(ctx, "resurrect", current.Version); ok {
		t.Fatal("resurrected removed document")
	}
}
