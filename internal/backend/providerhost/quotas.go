package providerhost

import (
	"context"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// AccountQuotas holds account snapshots and owns their pending storage writes.
type AccountQuotas struct {
	vault *Vault
	// mu protects membership and shutdown admission, never storage IO.
	mu      sync.Mutex
	held    map[quotaAccount]*accountQuota
	closed  bool
	workers sync.WaitGroup
}
type quotaAccount struct {
	entry   webapiproto.ProviderID
	account webapiproto.CredentialID
}
type accountQuota struct {
	owner     *AccountQuotas
	key       quotaAccount
	snapshots provider.MemorySnapshots
	// wake coalesces writes; one worker serializes storage writes for this account.
	wake chan struct{}
	stop chan struct{}
}

// NewAccountQuotas creates the owner of account quota writes.
func NewAccountQuotas(vault *Vault) *AccountQuotas {
	return &AccountQuotas{vault: vault, held: make(map[quotaAccount]*accountQuota)}
}

// Store returns the snapshot store for an account.
func (q *AccountQuotas) Store(
	id webapiproto.ProviderID,
	record database.CredentialRow,
) provider.QuotaSnapshotStore {
	key := quotaAccount{id, record.ID}
	q.mu.Lock()
	defer q.mu.Unlock()
	if existing := q.held[key]; existing != nil {
		return existing
	}
	s := &accountQuota{
		owner: q,
		key:   key,
		wake:  make(chan struct{}, 1),
		stop:  make(chan struct{}),
	}
	if record.Quota != nil {
		s.snapshots.Update(func(*types.QuotaSnapshot) types.QuotaSnapshot { return *record.Quota })
	}
	q.held[key] = s
	if !q.closed {
		q.workers.Go(s.run)
	} else {
		close(s.stop)
	}
	return s
}

// Latest returns the latest account snapshot, if any.
// Latest returns an independent copy of the account’s latest quota snapshot.
func (q *AccountQuotas) Latest(id webapiproto.ProviderID, record database.CredentialRow) *types.QuotaSnapshot {
	q.mu.Lock()
	held := q.held[quotaAccount{id, record.ID}]
	q.mu.Unlock()
	if held != nil {
		return held.Latest()
	}
	var snapshot provider.MemorySnapshots
	if record.Quota == nil {
		return nil
	}
	return snapshot.Update(func(*types.QuotaSnapshot) types.QuotaSnapshot { return *record.Quota })
}

// ForgetAccount forgets one account snapshot.
func (q *AccountQuotas) ForgetAccount(id webapiproto.ProviderID, account webapiproto.CredentialID) {
	q.mu.Lock()
	defer q.mu.Unlock()
	key := quotaAccount{id, account}
	if s := q.held[key]; s != nil {
		delete(q.held, key)
		if !q.closed {
			close(s.stop)
		}
	}
}

// ForgetEntry forgets all snapshots of an entry.
func (q *AccountQuotas) ForgetEntry(id webapiproto.ProviderID) {
	q.mu.Lock()
	defer q.mu.Unlock()
	for key, s := range q.held {
		if key.entry == id {
			delete(q.held, key)
			if !q.closed {
				close(s.stop)
			}
		}
	}
}

// Close waits for pending quota writes.
func (q *AccountQuotas) Close(_ context.Context) error {
	q.mu.Lock()
	if !q.closed {
		q.closed = true
		for _, s := range q.held {
			close(s.stop)
		}
	}
	q.mu.Unlock()
	// Storage writes are commits: join them even if the cleanup caller is canceled.
	q.workers.Wait()
	return nil
}

// Latest returns an independent copy of the account’s latest quota snapshot.
func (s *accountQuota) Latest() *types.QuotaSnapshot { return s.snapshots.Latest() }

// Update publishes the account snapshot and wakes its storage worker.
func (s *accountQuota) Update(next func(*types.QuotaSnapshot) types.QuotaSnapshot) *types.QuotaSnapshot {
	result := s.snapshots.Update(next)
	select {
	case s.wake <- struct{}{}:
	default:
	}
	return result
}

func (s *accountQuota) run() {
	for {
		select {
		case <-s.wake:
			s.write()
		case <-s.stop:
			select {
			case <-s.wake:
				s.write()
			default:
			}
			return
		}
	}
}

func (s *accountQuota) write() {
	s.owner.mu.Lock()
	current := s.owner.held[s.key] == s
	s.owner.mu.Unlock()
	if !current {
		return
	}
	snapshot := s.Latest()
	if snapshot == nil {
		return
	}
	ctx := context.Background()
	if err := s.owner.vault.control.SetCredentialQuota(ctx, s.key.entry, s.key.account, *snapshot); err != nil {
		slog.Warn("an account's quota snapshot was not stored; the next probe reads it again", "error", err)
		return
	}
	s.owner.vault.MarkEntryChanged(ctx, s.key.entry)
}
