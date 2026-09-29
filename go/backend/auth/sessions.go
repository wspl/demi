package auth

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"time"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type WebSessions struct{ Control *storage.Control }
type OpenedSession struct {
	Token     string
	ExpiresAt core.Timestamp
}

func (s WebSessions) Open(ctx context.Context, user webapi.UserID) (OpenedSession, error) {
	bytes := make([]byte, 32)
	if _, err := rand.Read(bytes); err != nil {
		return OpenedSession{}, err
	}
	token := hex.EncodeToString(bytes)
	expiry, err := s.Control.OpenWebSession(ctx, storage.HashToken(token), user, storage.SessionPolicy{Lifetime: 30 * 24 * time.Hour, RenewBelow: 15 * 24 * time.Hour})
	if err != nil {
		return OpenedSession{}, err
	}
	return OpenedSession{token, expiry}, nil
}
func (s WebSessions) Resolve(ctx context.Context, token string) (*storage.ResolvedSession, error) {
	return s.Control.ResolveWebSession(ctx, storage.HashToken(token), storage.SessionPolicy{Lifetime: 30 * 24 * time.Hour, RenewBelow: 15 * 24 * time.Hour})
}
func (s WebSessions) Check(ctx context.Context, hash storage.TokenHash) (*storage.ResolvedSession, error) {
	return s.Control.ResolveWebSession(ctx, hash, storage.SessionPolicy{})
}
func (s WebSessions) Close(ctx context.Context, token string) error {
	return s.Control.CloseWebSession(ctx, storage.HashToken(token))
}
