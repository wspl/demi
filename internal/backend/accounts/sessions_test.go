package accounts

import (
	"context"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/database"
)

type checkingSessionStore struct {
	SessionStore
	policy database.SessionPolicy
}

func (s *checkingSessionStore) ResolveWebSession(_ context.Context, _ database.TokenHash, policy database.SessionPolicy) (*database.ResolvedSession, error) {
	s.policy = policy
	return &database.ResolvedSession{Renewed: false}, nil
}

func TestSynchronizationCheckNeverRenews(t *testing.T) {
	store := &checkingSessionStore{}
	session, err := NewWebSessions(store).Check(t.Context(), database.TokenHash{})
	if err != nil || session == nil || session.Renewed {
		t.Fatalf("check: %+v %v", session, err)
	}
	if store.policy.RenewBelow != 0 || store.policy.Lifetime != 30*24*time.Hour {
		t.Fatalf("check policy: %+v", store.policy)
	}
}
