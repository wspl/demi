package backend

import (
	"context"
	"errors"
	"fmt"
	"unicode/utf8"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/gates"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// errEntryGone refuses a write to the accounts of an entry deleted
// meanwhile.
var errEntryGone = errors.New("the provider entry no longer exists")

// poolStoreError is a failure of the pool's store, the Rust's
// PoolError::Store: an operation over the pool that fails with it failed
// for the backend, not for what its caller gave it. Its message names
// tables and columns, never a secret.
type poolStoreError struct{ err error }

func (e *poolStoreError) Error() string { return e.err.Error() }
func (e *poolStoreError) Unwrap() error { return e.err }

// storeFailure is err as a failure of the pool's store, or nil.
func storeFailure(err error) error {
	if err == nil {
		return nil
	}
	return &poolStoreError{err}
}

// vaultPool is the vault's credential pool (providers.md § The credential
// pool contract): one entry's accounts as records of the control store, each
// secret sealed to its row and versioned by every write, with the vault's
// refresh turns. A pool is bound to its entry, so a provider given it cannot
// reach another entry's accounts. Every failure of its store is a
// poolStoreError.
type vaultPool struct {
	vault    *Vault
	provider webapi.ProviderID
}

// accountMeta is an account's public metadata from its record.
func accountMeta(row storage.CredentialRow) provider.AccountMeta {
	return provider.AccountMeta{
		ID:          row.ID.String(),
		Label:       row.Label,
		Detail:      row.Detail,
		UpdatedAt:   row.UpdatedAt,
		Source:      row.Source,
		IdentityKey: row.IdentityKey,
	}
}

func (p *vaultPool) List(ctx context.Context) ([]provider.AccountMeta, error) {
	rows, err := p.vault.Accounts(ctx, p.provider)
	if err != nil {
		return nil, storeFailure(err)
	}
	accounts := make([]provider.AccountMeta, 0, len(rows))
	for _, row := range rows {
		accounts = append(accounts, accountMeta(row))
	}
	return accounts, nil
}

func (p *vaultPool) Meta(ctx context.Context, id string) (*provider.AccountMeta, error) {
	account, err := webapi.ParseCredentialID(id)
	if err != nil {
		// An id that is no account's names nothing.
		return nil, nil
	}
	row, err := p.vault.Account(ctx, p.provider, account)
	if err != nil || row == nil {
		return nil, storeFailure(err)
	}
	meta := accountMeta(*row)
	return &meta, nil
}

func (p *vaultPool) Active(ctx context.Context) (*string, error) {
	row, err := p.vault.control.Provider(ctx, p.provider)
	if err != nil || row == nil || row.Active == nil {
		return nil, storeFailure(err)
	}
	active := row.Active.String()
	return &active, nil
}

func (p *vaultPool) SetActive(ctx context.Context, id string) error {
	account, err := webapi.ParseCredentialID(id)
	if err != nil {
		return provider.PoolNotFound{ID: id}
	}
	selected, err := p.vault.control.SetActiveCredential(ctx, p.provider, account)
	if err != nil {
		return storeFailure(err)
	}
	if !selected {
		return provider.PoolNotFound{ID: id}
	}
	p.vault.markEntryChanged(ctx, p.provider)
	return nil
}

// Write stores the account, and selects it when the entry has no active
// account, in one transaction; the record's time is the write's.
func (p *vaultPool) Write(ctx context.Context, meta provider.AccountMeta, secret string) error {
	account, err := webapi.ParseCredentialID(meta.ID)
	if err != nil {
		return storeFailure(err)
	}
	stored, err := p.vault.control.WriteCredential(ctx, p.provider, storage.CredentialWrite{
		ID:          account,
		IdentityKey: meta.IdentityKey,
		Label:       meta.Label,
		Detail:      meta.Detail,
		Source:      meta.Source,
		Secret:      p.vault.sealSecret(p.provider, account, secret),
	})
	if err != nil {
		return storeFailure(err)
	}
	if !stored {
		return storeFailure(errEntryGone)
	}
	p.vault.markEntryChanged(ctx, p.provider)
	return nil
}

func (p *vaultPool) Document(id string) provider.AccountDocument {
	document := &vaultDocument{vault: p.vault, provider: p.provider, name: "account " + id}
	if account, err := webapi.ParseCredentialID(id); err == nil {
		document.account = &account
	}
	return document
}

func (p *vaultPool) Remove(ctx context.Context, id string) error {
	account, err := webapi.ParseCredentialID(id)
	if err != nil {
		// An id that is no account's names nothing to remove.
		return nil
	}
	if err := p.vault.control.RemoveCredential(ctx, p.provider, account); err != nil {
		return storeFailure(err)
	}
	p.vault.markEntryChanged(ctx, p.provider)
	return nil
}

// vaultDocument is one account's secret document in the vault.
type vaultDocument struct {
	vault    *Vault
	provider webapi.ProviderID
	// account is nil for an id that is no account's, whose document is
	// empty.
	account *webapi.CredentialID
	name    string
}

func (d *vaultDocument) Name() string { return d.name }

// Read is the account's document. A secret that does not open is corrupt:
// requests with the account fail with an authentication error, and nothing
// replaces it.
func (d *vaultDocument) Read(ctx context.Context) (*provider.Revision, error) {
	if d.account == nil {
		return nil, nil
	}
	row, err := d.vault.Account(ctx, d.provider, *d.account)
	if err != nil || row == nil {
		return nil, storeFailure(err)
	}
	document, err := d.vault.key.Open(SecretRow(d.provider, *d.account), row.Secret)
	if err != nil {
		return nil, storeFailure(fmt.Errorf("the secret of %s does not open", d.name))
	}
	if !utf8.Valid(document) {
		return nil, storeFailure(fmt.Errorf("the secret of %s is not UTF-8", d.name))
	}
	return &provider.Revision{Text: string(document), Version: row.Version}, nil
}

func (d *vaultDocument) Replace(ctx context.Context, text string, version uint64) (bool, error) {
	if d.account == nil {
		return false, nil
	}
	sealed := d.vault.sealSecret(d.provider, *d.account, text)
	replaced, err := d.vault.control.ReplaceCredentialSecret(ctx, d.provider, *d.account, sealed, version)
	if err != nil {
		return false, storeFailure(err)
	}
	if replaced {
		d.vault.markEntryChanged(ctx, d.provider)
	}
	return replaced, nil
}

// RefreshTurn waits for the account's turn to refresh. A provider id is a
// UUID, which holds no "/", so no two accounts of any entries share a key.
func (d *vaultDocument) RefreshTurn(ctx context.Context) (*gates.KeyedPermit[string], error) {
	key := d.provider.String() + "/"
	if d.account != nil {
		key += d.account.String()
	}
	return d.vault.gates.Acquire(ctx, key)
}
