package accounts

import (
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/core"
)

// accountClock moves account record expiry time without wall-time waits or virtualized IO.
type accountClock struct{ at core.Timestamp }

func (c *accountClock) Now() core.Timestamp { return c.at }

func TestWebSessionLifecycle(t *testing.T) {
	clock := &accountClock{at: "2026-01-01T00:00:00.000Z"}
	control := databasetest.Control(t.Context(), t, clock)
	user := databasetest.Master(t.Context(), t, control)
	sessions := NewWebSessions(control)
	opened, err := sessions.Open(t.Context(), user.ID)
	if err != nil {
		t.Fatal(err)
	}
	if opened.ExpiresAt != "2026-01-31T00:00:00.000Z" {
		t.Fatalf("initial expiry: %s", opened.ExpiresAt)
	}
	// Exactly fifteen days remaining does not renew; less than fifteen does.
	clock.at = "2026-01-16T00:00:00.000Z"
	resolved, found, err := sessions.Resolve(t.Context(), opened.Token)
	if err != nil || !found || resolved.User.ID != user.ID || resolved.Renewed ||
		resolved.ExpiresAt != opened.ExpiresAt {
		t.Fatalf("resolve at renewal boundary: %+v, %v", resolved, err)
	}
	clock.at = "2026-01-17T00:00:00.000Z"
	checked, ok, err := sessions.Check(t.Context(), database.HashToken(opened.Token))
	if err != nil || !ok || checked.Renewed || checked.ExpiresAt != opened.ExpiresAt {
		t.Fatalf("synchronization check renewed: %+v, %v", checked, err)
	}
	resolved, found, err = sessions.Resolve(t.Context(), opened.Token)
	if err != nil || !found || !resolved.Renewed || resolved.ExpiresAt != "2026-02-16T00:00:00.000Z" {
		t.Fatalf("request renewal: %+v, %v", resolved, err)
	}
	if err := sessions.Close(t.Context(), opened.Token); err != nil {
		t.Fatal(err)
	}
	resolved, found, err = sessions.Resolve(t.Context(), opened.Token)
	if err != nil || found {
		t.Fatalf("closed session resolved: %+v, %v", resolved, err)
	}
	opened, err = sessions.Open(t.Context(), user.ID)
	if err != nil {
		t.Fatal(err)
	}
	clock.at = opened.ExpiresAt
	resolved, found, err = sessions.Resolve(t.Context(), opened.Token)
	if err != nil || found {
		t.Fatalf("expired session resolved: %+v, %v", resolved, err)
	}
}
