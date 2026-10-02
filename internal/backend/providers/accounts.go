//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// SetupTokenFamily is the family whose accounts are setup tokens.
const SetupTokenFamily = "claude-code"

// VaultCredentialPool is an entry's persisted, encrypted account pool.
type VaultCredentialPool struct{}

// AccountQuotas holds account snapshots and owns their pending storage writes.
type AccountQuotas struct{}

// ImportSetupToken creates the Claude Code entry and its first account in one transaction.
func ImportSetupToken(ctx context.Context, assembly *Assembly, owner webapi.UserID, label, token string) (ProviderEntry, error) {
	panic("not written: b-providers")
}

// AddToken adds a setup token account to an entry.
func AddToken(ctx context.Context, assembly *Assembly, entry ProviderEntry, token string) (core.AccountInfo, error) {
	panic("not written: b-providers")
}

// ListAccounts returns accounts and active selection only when disclose is true.
func ListAccounts(ctx context.Context, assembly *Assembly, entry ProviderEntry, disclose bool) (webapi.Accounts, error) {
	panic("not written: b-providers")
}

// ActivateAccount selects the account for subsequent requests.
func ActivateAccount(ctx context.Context, assembly *Assembly, entry ProviderEntry, account webapi.CredentialID) (webapi.CredentialID, error) {
	panic("not written: b-providers")
}

// RemoveAccount removes a non-active account and its quota snapshot.
func RemoveAccount(ctx context.Context, assembly *Assembly, entry ProviderEntry, account webapi.CredentialID) error {
	panic("not written: b-providers")
}

// NewVaultCredentialPool binds a credential pool to an entry.
func NewVaultCredentialPool(vault *Vault, id webapi.ProviderID) *VaultCredentialPool {
	panic("not written: b-providers")
}

// AccountMeta returns public metadata from an account record.
func AccountMeta(row database.CredentialRow) provider.AccountMeta { panic("not written: b-providers") }

// List returns public account metadata.
func (p *VaultCredentialPool) List(ctx context.Context) ([]provider.AccountMeta, error) {
	panic("not written: b-providers")
}

// Meta returns account metadata or nil.
func (p *VaultCredentialPool) Meta(ctx context.Context, id string) (*provider.AccountMeta, error) {
	panic("not written: b-providers")
}

// Active returns the selected account ID.
func (p *VaultCredentialPool) Active(ctx context.Context) (*string, error) {
	panic("not written: b-providers")
}

// SetActive selects an existing account.
func (p *VaultCredentialPool) SetActive(ctx context.Context, id string) error {
	panic("not written: b-providers")
}

// Write stores public metadata and seals its secret document.
func (p *VaultCredentialPool) Write(ctx context.Context, meta provider.AccountMeta, secret string) error {
	panic("not written: b-providers")
}

// Document returns versioned secret access with serialized refresh turns.
func (p *VaultCredentialPool) Document(id string) provider.AccountDocument {
	panic("not written: b-providers")
}

// Remove removes an account other than the active one.
func (p *VaultCredentialPool) Remove(ctx context.Context, id string) error {
	panic("not written: b-providers")
}

// NewAccountQuotas creates the owner of account quota writes.
func NewAccountQuotas(vault *Vault) *AccountQuotas { panic("not written: b-providers") }

// Store returns the snapshot store for an account.
func (q *AccountQuotas) Store(id webapi.ProviderID, record database.CredentialRow) provider.QuotaSnapshotStore {
	panic("not written: b-providers")
}

// Latest returns the latest account snapshot, if any.
func (q *AccountQuotas) Latest(id webapi.ProviderID, record database.CredentialRow) *core.QuotaSnapshot {
	panic("not written: b-providers")
}

// ForgetAccount forgets one account snapshot.
func (q *AccountQuotas) ForgetAccount(id webapi.ProviderID, account webapi.CredentialID) {
	panic("not written: b-providers")
}

// ForgetEntry forgets all snapshots of an entry.
func (q *AccountQuotas) ForgetEntry(id webapi.ProviderID) { panic("not written: b-providers") }

// Close waits for pending quota writes.
func (q *AccountQuotas) Close(ctx context.Context) error { panic("not written: b-providers") }
