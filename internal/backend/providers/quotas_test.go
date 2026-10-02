package providers

import (
	"context"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

func TestQuotaUpdatesMergeAndClosePersistsNewestWithoutSharingMutableData(t *testing.T) {
	vault, owner := vaultFixture(t)
	ctx := t.Context()
	entry, err := vault.CreateSubscription(ctx, owner.ID, "device", "Device", provider.NewMemoryCredentialPool())
	if err != nil {
		t.Fatal(err)
	}
	pool := vault.Pool(entry.ID)
	if err := pool.Write(ctx, testAccount("a"), "{}"); err != nil {
		t.Fatal(err)
	}
	row, err := vault.Account(ctx, entry.ID, "a")
	if err != nil {
		t.Fatal(err)
	}
	quotas := NewAccountQuotas(vault)
	store := quotas.Store(entry.ID, *row)
	var workers sync.WaitGroup
	for _, id := range []string{"probe", "observation"} {
		workers.Go(func() {
			store.Update(func(previous *core.QuotaSnapshot) core.QuotaSnapshot {
				next := core.QuotaSnapshot{ObservedAt: core.SystemClock{}.Now(), Source: "probe", Windows: []core.QuotaWindow{}}
				if previous != nil {
					next = *previous
				}
				next.Windows = append(next.Windows, core.QuotaWindow{ID: id, Label: id})
				return next
			})
		})
	}
	workers.Wait()
	snapshot := store.Latest()
	if len(snapshot.Windows) != 2 {
		t.Fatal(snapshot)
	}
	snapshot.Windows[0].ID = "caller mutation"
	if store.Latest().Windows[0].ID == "caller mutation" {
		t.Fatal("snapshot alias")
	}
	if err := quotas.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	row, err = vault.Account(ctx, entry.ID, "a")
	if err != nil || row.Quota == nil || len(row.Quota.Windows) != 2 {
		t.Fatal(row, err)
	}
	restarted := NewAccountQuotas(vault)
	defer func() {
		if err := restarted.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	if got := restarted.Latest(entry.ID, *row); got == nil || len(got.Windows) != 2 {
		t.Fatal(got)
	}
	held := restarted.Store(entry.ID, *row)
	if got := held.Latest(); got == nil || len(got.Windows) != 2 {
		t.Fatal(got)
	}
	restarted.ForgetAccount(entry.ID, "a")
	restarted.ForgetEntry(entry.ID)
}
