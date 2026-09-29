package backend

import (
	"context"
	"log/slog"
	"sync"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// quotaAccount names one account of one entry.
type quotaAccount struct {
	provider webapi.ProviderID
	account  webapi.CredentialID
}

// AccountQuotas are the quota snapshots of every subscription account this
// process uses (usage-and-quota.md § One snapshot per account): held in
// memory by the process that uses the account and written through to the
// account's record, so that a snapshot outlives a rebuilt provider and a
// restart. A write that fails is logged; it costs at most a later probe.
type AccountQuotas struct {
	vault *Vault
	// writes are the record writes; Close waits for them.
	writes *TaskGroup
	// mu guards held: probes at the edge and observations of the users'
	// requests update snapshots from any goroutine, and no section waits.
	mu sync.Mutex
	// held has an account from its first use; a nil snapshot holds none
	// yet. Snapshots are never changed, only replaced.
	held map[quotaAccount]*core.QuotaSnapshot
}

// NewAccountQuotas holds no snapshot yet.
func NewAccountQuotas(vault *Vault) *AccountQuotas {
	return &AccountQuotas{vault: vault, writes: newTaskGroup(context.Background()), held: map[quotaAccount]*core.QuotaSnapshot{}}
}

// Store is the snapshot store of the entry's account record, which starts
// from the record's snapshot the first time this process uses the account.
func (q *AccountQuotas) Store(id webapi.ProviderID, record storage.CredentialRow) provider.QuotaSnapshotStore {
	account := quotaAccount{id, record.ID}
	q.mu.Lock()
	defer q.mu.Unlock()
	if _, held := q.held[account]; !held {
		q.held[account] = record.Quota
	}
	return &accountQuota{quotas: q, account: account}
}

// Latest is the account's snapshot now: this process's newest, else its
// record's.
func (q *AccountQuotas) Latest(id webapi.ProviderID, record storage.CredentialRow) *core.QuotaSnapshot {
	q.mu.Lock()
	defer q.mu.Unlock()
	if held, ok := q.held[quotaAccount{id, record.ID}]; ok {
		return held
	}
	return record.Quota
}

// ForgetAccount forgets a removed account's snapshot.
func (q *AccountQuotas) ForgetAccount(id webapi.ProviderID, account webapi.CredentialID) {
	q.mu.Lock()
	defer q.mu.Unlock()
	delete(q.held, quotaAccount{id, account})
}

// ForgetEntry forgets the snapshots of a deleted entry's accounts.
func (q *AccountQuotas) ForgetEntry(id webapi.ProviderID) {
	q.mu.Lock()
	defer q.mu.Unlock()
	for account := range q.held {
		if account.provider == id {
			delete(q.held, account)
		}
	}
}

// Close waits for the record writes still running; it starts no more.
func (q *AccountQuotas) Close() {
	q.writes.close()
	q.writes.wait()
}

// writeThrough writes the account's newest snapshot to its record. A write
// stores whatever is newest when it runs, so writes that run out of order
// still leave the newest snapshot stored. A write runs to its end: Close
// waits for it rather than cutting it.
func (q *AccountQuotas) writeThrough(account quotaAccount) {
	q.writes.Go(func(ctx context.Context) {
		q.mu.Lock()
		newest := q.held[account]
		q.mu.Unlock()
		if newest == nil {
			return
		}
		ctx = context.WithoutCancel(ctx)
		if err := q.vault.control.SetCredentialQuota(ctx, account.provider, account.account, *newest); err != nil {
			slog.Warn("an account's quota snapshot was not stored; the next probe reads it again", "error", err)
			return
		}
		q.vault.markEntryChanged(ctx, account.provider)
	})
}

// accountQuota is one account's snapshot store.
type accountQuota struct {
	quotas  *AccountQuotas
	account quotaAccount
}

func (a *accountQuota) Latest() *core.QuotaSnapshot {
	a.quotas.mu.Lock()
	defer a.quotas.mu.Unlock()
	return a.quotas.held[a.account]
}

// Update publishes next's snapshot, made from the one held, and writes it
// through to the account's record.
func (a *accountQuota) Update(next func(*core.QuotaSnapshot) core.QuotaSnapshot) *core.QuotaSnapshot {
	a.quotas.mu.Lock()
	snapshot := next(a.quotas.held[a.account])
	a.quotas.held[a.account] = &snapshot
	a.quotas.mu.Unlock()
	a.quotas.writeThrough(a.account)
	return &snapshot
}
