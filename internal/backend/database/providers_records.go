package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ProviderRow is a `providers` row: an entry with its configuration still sealed.
type ProviderRow struct {
	ID     webapi.ProviderID
	Owner  webapi.UserID
	Family string
	Kind   webapi.CredentialKind
	Label  string
	// An API-key entry's sealed configuration; a subscription entry has
	// none.
	Config *[]byte
	// A subscription entry's active account.
	Active    *webapi.CredentialID
	CreatedAt core.Timestamp
}

// NewProvider is an entry as it is first stored.
type NewProvider struct {
	ID     webapi.ProviderID
	Owner  webapi.UserID
	Family string
	Kind   webapi.CredentialKind
	Label  string
	Config *[]byte
	Active *webapi.CredentialID
}

// CredentialRow is a `provider_credentials` row: one account with its secret still sealed.
type CredentialRow struct {
	ID          webapi.CredentialID
	IdentityKey *string
	Label       string
	Detail      *string
	Source      string
	Secret      []byte
	// Advanced by every write of the secret.
	Version uint64
	Quota   *core.QuotaSnapshot
	// When the account was last stored or refreshed.
	UpdatedAt core.Timestamp
}

// CredentialWrite is an account as a write stores it; the record's time is the write's.
type CredentialWrite struct {
	ID          webapi.CredentialID
	IdentityKey *string
	Label       string
	Detail      *string
	Source      string
	Secret      []byte
}
