package database

import (
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// ProviderRow is a `providers` row: an entry with its configuration still sealed.
type ProviderRow struct {
	ID     webapiproto.ProviderID
	Owner  webapiproto.UserID
	Family string
	Kind   webapiproto.CredentialKind
	Label  string
	// An API-key entry's sealed configuration; a subscription entry has
	// none.
	Config *[]byte
	// A subscription entry's active account.
	Active    *webapiproto.CredentialID
	CreatedAt types.Timestamp
}

// NewProvider is an entry as it is first stored.
type NewProvider struct {
	ID     webapiproto.ProviderID
	Owner  webapiproto.UserID
	Family string
	Kind   webapiproto.CredentialKind
	Label  string
	Config *[]byte
	Active *webapiproto.CredentialID
}

// CredentialRow is a `provider_credentials` row: one account with its secret still sealed.
type CredentialRow struct {
	ID          webapiproto.CredentialID
	IdentityKey *string
	Label       string
	Detail      *string
	Source      string
	Secret      []byte
	// Advanced by every write of the secret.
	Version uint64
	Quota   *types.QuotaSnapshot
	// When the account was last stored or refreshed.
	UpdatedAt types.Timestamp
}

// CredentialWrite is an account as a write stores it; the record's time is the write's.
type CredentialWrite struct {
	ID          webapiproto.CredentialID
	IdentityKey *string
	Label       string
	Detail      *string
	Source      string
	Secret      []byte
}
