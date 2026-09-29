package backend

import (
	"github.com/wspl/demi/go/backend/auth"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
)

// Services is what the users share, safe from any goroutine: the program
// (package edge's start) builds it once and hands it to the shards
// (g7-backend.md § Services). The vault, the model access, the managed
// Clouds and the synchronization registry join it with their parts.
type Services struct {
	// Clock is the wall clock the backend reads times from.
	Clock core.Clock
	// Control is the control database: users, sessions, providers, devices
	// and the rest of what is not one conversation's.
	Control *storage.Control
	// Conversations are the conversation databases.
	Conversations *storage.ConversationStores
	// Blobs are the stored media and their references.
	Blobs *storage.BlobStores
	// Hasher hashes and verifies passwords.
	Hasher *auth.PasswordHasher
	// Sessions are the web sessions over Control.
	Sessions *auth.WebSessions
	// Limiter spaces failed logins.
	Limiter *auth.LoginLimiter
	// Email changes a user's address through a mailed code.
	Email *auth.EmailChanges
}
