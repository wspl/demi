package backend

import (
	"github.com/wspl/demi/go/backend/auth"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
)

// Services is what the users share, safe from any goroutine: the program
// (package edge's start) builds it once and hands it to the shards
// (g7-backend.md § Services). The Clouds' settings join it with the Cloud's
// machine.
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
	// Sync is the registry of the pages' open synchronization channels.
	Sync *SyncRegistry
	// Vault holds the provider entries and their accounts.
	Vault *Vault
	// Assembly builds the providers of the entries and reads their
	// catalogs.
	Assembly *ProviderAssembly
	// ClaudeReleases are the vendor's Claude Code releases, which the CLI
	// on each Cloud follows.
	ClaudeReleases *ClaudeReleases
	// Operations let one change of an entry run at a time.
	Operations *ProviderOperations
	// Logins are the device logins under way.
	Logins *LoginFlows
	// Machines is the machine manager's client, which every Cloud's
	// operations go through.
	Machines *MachinesClient
	// CloudCapacity bounds the Clouds that are not stopped, across users.
	CloudCapacity *CloudCapacity
}
