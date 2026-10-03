package providers

import (
	"bytes"
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

func vaultFixture(t *testing.T) (*Vault, webapi.UserDTO) {
	t.Helper()
	control := databasetest.Control(t.Context(), t, core.SystemClock{})
	owner := databasetest.Master(t.Context(), t, control)
	return NewVault(
		control,
		NewVaultKey(databasetest.Key()),
		webapi.InstanceModeIsolated,
		&pagesync.SyncRegistry{},
	), owner
}

func testAccount(id string) provider.AccountMeta {
	return provider.AccountMeta{
		ID:          id,
		Label:       id + "@example.test",
		UpdatedAt:   core.UnixEpoch,
		Source:      "login:device",
		IdentityKey: &id,
	}
}

// Cost: temporary SQLite only. Covers the persisted pool at the database boundary.
func TestAccountIsSealedAndRefreshedOnlyOverItsReadVersion(t *testing.T) {
	ctx := t.Context()
	vault, owner := vaultFixture(t)
	staged := provider.NewMemoryCredentialPool()
	entry, err := vault.CreateSubscription(ctx, owner.ID, "codex", "Codex", staged)
	if err != nil {
		t.Fatal(err)
	}
	pool := vault.Pool(entry.ID)
	if _, ok, err := pool.Active(ctx); err != nil || ok {
		t.Fatalf("initial active: %v %v", ok, err)
	}
	if err := pool.Write(ctx, testAccount("a"), `{"access":"test","refresh":"one"}`); err != nil {
		t.Fatal(err)
	}
	if active, ok, err := pool.Active(ctx); err != nil || !ok || active != "a" {
		t.Fatalf("first active: %v %v", active, err)
	}
	rows, err := vault.Accounts(ctx, entry.ID)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(rows[0].Secret, []byte("one")) {
		t.Fatal("plaintext stored")
	}
	doc := pool.Document("a")
	first, _, err := doc.Read(ctx)
	if err != nil {
		t.Fatal(err)
	}
	second, _, err := doc.Read(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if first.Text != `{"access":"test","refresh":"one"}` {
		t.Fatal(first.Text)
	}
	if ok, err := doc.Replace(ctx, `{"access":"test","refresh":"two"}`, first.Version); err != nil || !ok {
		t.Fatalf("replace: %v %v", ok, err)
	}
	if ok, err := doc.Replace(ctx, `{"access":"test","refresh":"lost"}`, second.Version); err != nil || ok {
		t.Fatalf("stale replace: %v %v", ok, err)
	}
	now, _, err := doc.Read(ctx)
	if err != nil || now.Text != `{"access":"test","refresh":"two"}` {
		t.Fatalf("read: %v %v", now, err)
	}
	var asked atomic.Int32
	var workers sync.WaitGroup
	results := make(chan string, 2)
	for range 2 {
		workers.Go(func() {
			value, err := provider.Renew(
				ctx,
				pool.Document("a"),
				providertest.DecodeTokenDocument,
				func(v providertest.TokenDocument) bool { return v.Refresh == "two" },
				func(_ context.Context, v providertest.TokenDocument) (providertest.TokenDocument, error) {
					asked.Add(1)
					v.Refresh += "+"
					return v, nil
				},
			)
			if err != nil {
				t.Error(err)
				return
			}
			results <- value.Refresh
		})
	}
	workers.Wait()
	close(results)
	for result := range results {
		if result != "two+" {
			t.Fatal(result)
		}
	}
	if asked.Load() != 1 {
		t.Fatal("duplicate renewal", asked.Load())
	}
	other, err := vault.CreateSubscription(ctx, owner.ID, "grok-build", "Grok", staged)
	if err != nil {
		t.Fatal(err)
	}
	foreign := vault.Pool(other.ID)
	if _, ok, err := foreign.Document("a").Read(ctx); err != nil || ok {
		t.Fatalf("foreign read: %v %v", ok, err)
	}
	if values, err := foreign.List(ctx); err != nil || len(values) != 0 {
		t.Fatalf("foreign list: %v %v", values, err)
	}
	if err := foreign.Write(ctx, testAccount("a"), "{}"); err != nil {
		t.Fatal(err)
	}
	databasetest.Execute(
		ctx,
		t,
		vault.control,
		"UPDATE provider_credentials SET secret = (SELECT secret FROM "+
			"provider_credentials WHERE provider_id = ?1 AND id = 'a') WHERE "+
			"provider_id = ?2",
		string(entry.ID),
		string(other.ID),
	)
	var poolErr *provider.PoolError
	if _, _, err := foreign.Document("a").Read(ctx); !errors.As(err, &poolErr) {
		t.Fatalf("copied ciphertext: %v", err)
	}
	if err := pool.Write(ctx, testAccount("b"), "{}"); err != nil {
		t.Fatal(err)
	}
	if err := pool.SetActive(ctx, "missing"); !errors.Is(err, provider.ErrNoAccount) {
		t.Fatalf("missing account: %v", err)
	}
	if err := pool.SetActive(ctx, "b"); err != nil {
		t.Fatal(err)
	}
	if err := pool.Remove(ctx, "b"); err != nil {
		t.Fatal(err)
	}
	if _, ok, err := pool.Active(ctx); err != nil || ok {
		t.Fatalf("removed active: %v %v", ok, err)
	}
	if err := vault.Delete(ctx, *entry); err != nil {
		t.Fatal(err)
	}
	if rows, err := vault.Accounts(ctx, entry.ID); err != nil || len(rows) != 0 {
		t.Fatalf("deleted accounts: %v %v", rows, err)
	}
}
