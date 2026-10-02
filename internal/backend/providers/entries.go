//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// Vault is the credential vault. Share its pointer across owners.
type Vault struct{}

// ProviderEntry is a provider entry, read and decoded.
type ProviderEntry struct {
	ID    webapi.ProviderID
	Owner webapi.UserID
	// The entry's family, such as `anthropic` or `codex`.
	Family     string
	Label      string
	Credential EntryCredential
	CreatedAt  core.Timestamp
}

// EntryCredential is how an entry authenticates.
type EntryCredential interface{ entryCredential() }

// SubscriptionCredential holds the active account of a subscription entry.
type SubscriptionCredential struct{ Active *webapi.CredentialID }

func (*SubscriptionCredential) entryCredential() {}
func (*APIKeyConfig) entryCredential()           {}

// An API-key entry's configuration: the document the vault seals. A field
// it does not name is an error, so a misspelled setting is reported rather
// than ignored.
// +demi:root
//
//nolint:revive // Contract documentation is copied verbatim from Rust.
type APIKeyConfig struct {
	APIKey  provider.Secret     `json:"apiKey"`
	BaseURL *webapi.EndpointURL `json:"baseUrl,omitempty"`
	WireAPI *core.WireAPI       `json:"wireApi,omitempty"`
	// The models.dev vendor the entry was added from.
	// +demi:length chars min=1
	VendorID *string                  `json:"vendorId,omitempty"`
	Models   *webapi.ConfiguredModels `json:"models,omitempty"`
}

// Kind returns the entry credential kind.
func (e ProviderEntry) Kind() webapi.CredentialKind { panic("not written: b-providers") }

// Active returns a subscription entry's active account.
func (e ProviderEntry) Active() *webapi.CredentialID { panic("not written: b-providers") }

// DTO returns the entry as the web app sees it: never its key.
func (e ProviderEntry) DTO() webapi.ProviderDTO { panic("not written: b-providers") }

// NewVault creates a vault with instance scope and page notifications.
func NewVault(control *database.ControlService, key *VaultKey, mode webapi.InstanceMode, sync *pagesync.SyncRegistry) *Vault {
	panic("not written: b-providers")
}

// Control returns the control store.
func (v *Vault) Control() *database.ControlService { panic("not written: b-providers") }

// Key returns the vault encryption key.
func (v *Vault) Key() *VaultKey { panic("not written: b-providers") }

// Gates returns the account refresh gates.
func (v *Vault) Gates() *provider.RefreshGates { panic("not written: b-providers") }

// Configures reports whether user configures entries of their scope.
func (v *Vault) Configures(user webapi.UserDTO) bool { panic("not written: b-providers") }

// OwnerFor resolves whose entries user infers with.
func (v *Vault) OwnerFor(ctx context.Context, user webapi.UserID) (webapi.UserID, error) {
	panic("not written: b-providers")
}

// MarkChanged marks providers changed for every user who infers with owner entries.
func (v *Vault) MarkChanged(owner webapi.UserID) { panic("not written: b-providers") }

// MarkEntryChanged marks the entry changed; lookup failures are logged.
func (v *Vault) MarkEntryChanged(ctx context.Context, id webapi.ProviderID) {
	panic("not written: b-providers")
}

// Visible returns the entry when it belongs to user scope, or nil.
func (v *Vault) Visible(ctx context.Context, user webapi.UserID, id webapi.ProviderID) (*ProviderEntry, error) {
	panic("not written: b-providers")
}

// Entries returns owner entries, oldest first.
func (v *Vault) Entries(ctx context.Context, owner webapi.UserID) ([]ProviderEntry, error) {
	panic("not written: b-providers")
}

// Entry reads and decodes an entry, or returns nil.
func (v *Vault) Entry(ctx context.Context, id webapi.ProviderID) (*ProviderEntry, error) {
	panic("not written: b-providers")
}

// CreateAPIKey stores a new API-key entry.
func (v *Vault) CreateAPIKey(ctx context.Context, owner webapi.UserID, family, label string, config APIKeyConfig) (ProviderEntry, error) {
	panic("not written: b-providers")
}

// CreateSubscription publishes the entry and staged accounts atomically; nil means it already exists.
func (v *Vault) CreateSubscription(ctx context.Context, owner webapi.UserID, family, label string, staged *provider.MemoryCredentialPool) (*ProviderEntry, error) {
	panic("not written: b-providers")
}

// Update replaces label or configuration; nil means the entry no longer exists.
func (v *Vault) Update(ctx context.Context, id webapi.ProviderID, label *string, config *APIKeyConfig) (*ProviderEntry, error) {
	panic("not written: b-providers")
}

// Delete deletes the entry and its accounts.
func (v *Vault) Delete(ctx context.Context, entry ProviderEntry) error {
	panic("not written: b-providers")
}

// Pool returns the credential pool bound to an entry.
func (v *Vault) Pool(id webapi.ProviderID) provider.CredentialPool { panic("not written: b-providers") }

// Accounts reads an entry account records.
func (v *Vault) Accounts(ctx context.Context, id webapi.ProviderID) ([]database.CredentialRow, error) {
	panic("not written: b-providers")
}

// Account reads an account record, or nil.
func (v *Vault) Account(ctx context.Context, id webapi.ProviderID, account webapi.CredentialID) (*database.CredentialRow, error) {
	panic("not written: b-providers")
}

// SealSecret seals a secret document for its account row.
func (v *Vault) SealSecret(id webapi.ProviderID, account webapi.CredentialID, secret string) ([]byte, error) {
	panic("not written: b-providers")
}
