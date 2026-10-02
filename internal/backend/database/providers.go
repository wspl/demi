package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Master returns the master account, which a shared instance's entries belong to.
func (c *ControlService) Master(ctx context.Context) (*webapi.UserID, error) {
	panic("not written: b-database")
}

// InsertProvider stores a new entry with its first accounts in one transaction. nil
// when the owner already holds the family's subscription entry: then
// nothing is stored.
func (c *ControlService) InsertProvider(ctx context.Context, provider NewProvider, accounts []CredentialWrite) (*ProviderRow, error) {
	panic("not written: b-database")
}

// Provider returns the entry with id, or nil when absent.
func (c *ControlService) Provider(ctx context.Context, id webapi.ProviderID) (*ProviderRow, error) {
	panic("not written: b-database")
}

// Providers returns the owner's entries, oldest first.
func (c *ControlService) Providers(ctx context.Context, owner webapi.UserID) ([]ProviderRow, error) {
	panic("not written: b-database")
}

// UpdateProvider replaces the entry's label or sealed configuration, and answers the
// entry as it now is; nil for an entry that no longer exists.
func (c *ControlService) UpdateProvider(ctx context.Context, id webapi.ProviderID, label *string, config *[]byte) (*ProviderRow, error) {
	panic("not written: b-database")
}

// DeleteProvider deletes the entry with its accounts and catalog record.
func (c *ControlService) DeleteProvider(ctx context.Context, id webapi.ProviderID) error {
	panic("not written: b-database")
}

// Credentials returns the entry's accounts, ordered by id.
func (c *ControlService) Credentials(ctx context.Context, provider webapi.ProviderID) ([]CredentialRow, error) {
	panic("not written: b-database")
}

// Credential returns the entry's account with id, or nil when absent.
func (c *ControlService) Credential(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID) (*CredentialRow, error) {
	panic("not written: b-database")
}

// WriteCredential inserts the account, or replaces the one with its id and advances its
// version, and selects it when the entry has no active account, all in
// one transaction; `false` for an entry that no longer exists.
func (c *ControlService) WriteCredential(ctx context.Context, provider webapi.ProviderID, account CredentialWrite) (bool, error) {
	panic("not written: b-database")
}

// ReplaceCredentialSecret stores a refreshed secret only while the account is still at
// `version`; `false` when another writer stored first.
func (c *ControlService) ReplaceCredentialSecret(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID, secret []byte, version uint64) (bool, error) {
	panic("not written: b-database")
}

// SetCredentialQuota replaces the account's kept quota snapshot; an account removed
// meanwhile keeps nothing.
func (c *ControlService) SetCredentialQuota(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID, quota core.QuotaSnapshot) error {
	panic("not written: b-database")
}

// RemoveCredential removes the account, and the entry's selection of it with it.
func (c *ControlService) RemoveCredential(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID) error {
	panic("not written: b-database")
}

// SetActiveCredential selects the account the entry infers with; `false` when the entry
// holds no such account.
func (c *ControlService) SetActiveCredential(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID) (bool, error) {
	panic("not written: b-database")
}

// CatalogRecord returns the entry's catalog record, validated.
func (c *ControlService) CatalogRecord(ctx context.Context, provider webapi.ProviderID) (*CatalogRecord, error) {
	panic("not written: b-database")
}

// PutCatalogRecord stores the entry's catalog record in place of the last one; an entry
// deleted meanwhile keeps nothing.
func (c *ControlService) PutCatalogRecord(ctx context.Context, provider webapi.ProviderID, record CatalogRecord) error {
	panic("not written: b-database")
}

// DeleteCatalogRecord removes the entry's cached catalog.
func (c *ControlService) DeleteCatalogRecord(ctx context.Context, provider webapi.ProviderID) error {
	panic("not written: b-database")
}
