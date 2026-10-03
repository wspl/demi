package accounts

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// SessionStore is the control database's session boundary.
type SessionStore interface {
	OpenWebSession(context.Context, database.TokenHash, webapi.UserID, database.SessionPolicy) (core.Timestamp, error)
	ResolveWebSession(
		context.Context,
		database.TokenHash,
		database.SessionPolicy,
	) (database.ResolvedSession, bool, error)
	CloseWebSession(context.Context, database.TokenHash) error
}

// OpenedSession is a new session: the cookie token and its expiry.
type OpenedSession struct {
	Token     string
	ExpiresAt core.Timestamp
}

// WebSessions opens, resolves and closes sessions in the control database.
type WebSessions struct{ control SessionStore }

// NewWebSessions returns the web session service.
func NewWebSessions(control SessionStore) *WebSessions { return &WebSessions{control: control} }

func sessionPolicy() database.SessionPolicy {
	return database.SessionPolicy{Lifetime: 30 * 24 * time.Hour, RenewBelow: 15 * 24 * time.Hour}
}

// Open creates a session whose cookie contains a random 256-bit token.
func (s *WebSessions) Open(ctx context.Context, user webapi.UserID) (OpenedSession, error) {
	var random [32]byte
	rand.Read(random[:])
	token := hex.EncodeToString(random[:])
	expires, err := s.control.OpenWebSession(ctx, database.HashToken(token), user, sessionPolicy())
	if err != nil {
		return OpenedSession{}, err
	}
	return OpenedSession{Token: token, ExpiresAt: expires}, nil
}

// Resolve finds a live session and renews it when less than 15 days remain.
func (s *WebSessions) Resolve(ctx context.Context, token string) (database.ResolvedSession, bool, error) {
	return s.control.ResolveWebSession(ctx, database.HashToken(token), sessionPolicy())
}

// Check finds the live session a token hash names without renewing it.
func (s *WebSessions) Check(ctx context.Context, token database.TokenHash) (database.ResolvedSession, bool, error) {
	policy := sessionPolicy()
	policy.RenewBelow = 0
	return s.control.ResolveWebSession(ctx, token, policy)
}

// Close deletes the session named by a cookie token.
func (s *WebSessions) Close(ctx context.Context, token string) error {
	return s.control.CloseWebSession(ctx, database.HashToken(token))
}
