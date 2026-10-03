package providers

import (
	"context"
	"errors"
	"fmt"
	"unicode/utf8"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// VaultCredentialPool is an entry's persisted, encrypted account pool.
type VaultCredentialPool struct {
	vault *Vault
	id    webapi.ProviderID
}

// NewVaultCredentialPool binds a credential pool to an entry.
func NewVaultCredentialPool(vault *Vault, id webapi.ProviderID) *VaultCredentialPool {
	return &VaultCredentialPool{vault: vault, id: id}
}

// AccountMeta returns public metadata from an account record.
func AccountMeta(row database.CredentialRow) provider.AccountMeta {
	return provider.AccountMeta{
		ID:          string(row.ID),
		Label:       row.Label,
		Detail:      row.Detail,
		UpdatedAt:   row.UpdatedAt,
		Source:      row.Source,
		IdentityKey: row.IdentityKey,
	}
}

// List returns public account metadata.
func (p *VaultCredentialPool) List(ctx context.Context) ([]provider.AccountMeta, error) {
	rows, err := p.vault.Accounts(ctx, p.id)
	if err != nil {
		return nil, &provider.PoolError{Err: err}
	}
	items := make([]provider.AccountMeta, 0, len(rows))
	for _, row := range rows {
		items = append(items, AccountMeta(row))
	}
	return items, nil
}

// Meta returns account metadata; ok is false when there is none.
func (p *VaultCredentialPool) Meta(ctx context.Context, id string) (provider.AccountMeta, bool, error) {
	account, err := webapi.ParseCredentialID(id)
	if err != nil {
		return provider.AccountMeta{}, false, nil
	}
	row, err := p.vault.Account(ctx, p.id, account)
	if err != nil {
		return provider.AccountMeta{}, false, &provider.PoolError{Err: err}
	}
	if row == nil {
		return provider.AccountMeta{}, false, nil
	}
	return AccountMeta(*row), true, nil
}

// Active returns the selected account ID; ok is false when there is none.
func (p *VaultCredentialPool) Active(ctx context.Context) (string, bool, error) {
	row, err := p.vault.control.Provider(ctx, p.id)
	if err != nil {
		return "", false, &provider.PoolError{Err: err}
	}
	if row == nil || row.Active == nil {
		return "", false, nil
	}
	return string(*row.Active), true, nil
}

// SetActive selects an existing account.
func (p *VaultCredentialPool) SetActive(ctx context.Context, id string) error {
	account, err := webapi.ParseCredentialID(id)
	if err != nil {
		return fmt.Errorf("%w %s", provider.ErrNoAccount, id)
	}
	ctx = context.WithoutCancel(ctx)
	ok, err := p.vault.control.SetActiveCredential(ctx, p.id, account)
	if err != nil {
		return &provider.PoolError{Err: err}
	}
	if !ok {
		return fmt.Errorf("%w %s", provider.ErrNoAccount, id)
	}
	p.vault.MarkEntryChanged(ctx, p.id)
	return nil
}

// Write stores public metadata and seals its secret document.
func (p *VaultCredentialPool) Write(ctx context.Context, meta provider.AccountMeta, secret string) error {
	id, err := webapi.ParseCredentialID(meta.ID)
	if err != nil {
		return &provider.PoolError{Err: err}
	}
	sealed, err := p.vault.SealSecret(p.id, id, secret)
	if err != nil {
		return &provider.PoolError{Err: err}
	}
	ctx = context.WithoutCancel(ctx)
	ok, err := p.vault.control.WriteCredential(
		ctx,
		p.id,
		database.CredentialWrite{
			ID:          id,
			IdentityKey: meta.IdentityKey,
			Label:       meta.Label,
			Detail:      meta.Detail,
			Source:      meta.Source,
			Secret:      sealed,
		},
	)
	if err != nil {
		return &provider.PoolError{Err: err}
	}
	if !ok {
		return &provider.PoolError{Err: errors.New("the provider entry no longer exists")}
	}
	p.vault.MarkEntryChanged(ctx, p.id)
	return nil
}

// Document returns versioned secret access with serialized refresh turns.
func (p *VaultCredentialPool) Document(id string) provider.AccountDocument {
	return &vaultDocument{pool: p, id: id}
}

// Remove deletes the account; removing the selected account clears the selection.
// Product operations enforce the active-account refusal before calling the pool.
func (p *VaultCredentialPool) Remove(ctx context.Context, id string) error {
	account, err := webapi.ParseCredentialID(id)
	if err != nil {
		return nil
	}
	ctx = context.WithoutCancel(ctx)
	if err := p.vault.control.RemoveCredential(ctx, p.id, account); err != nil {
		return &provider.PoolError{Err: err}
	}
	p.vault.MarkEntryChanged(ctx, p.id)
	return nil
}

// vaultDocument provides versioned access to exactly one account's sealed document.
type vaultDocument struct {
	pool *VaultCredentialPool
	id   string
}

// Name identifies the account whose sealed document is read.
func (d *vaultDocument) Name() string { return "account " + d.id }

// Read opens the account’s sealed secret with its stored version; ok is false when there is none.
func (d *vaultDocument) Read(ctx context.Context) (provider.Revision, bool, error) {
	id, err := webapi.ParseCredentialID(d.id)
	if err != nil {
		return provider.Revision{}, false, nil
	}
	row, err := d.pool.vault.Account(ctx, d.pool.id, id)
	if err != nil {
		return provider.Revision{}, false, &provider.PoolError{Err: err}
	}
	if row == nil {
		return provider.Revision{}, false, nil
	}
	plain, err := d.pool.vault.key.Open(SecretRow{Provider: d.pool.id, Account: id}, row.Secret)
	if err != nil {
		return provider.Revision{}, false, &provider.PoolError{
			Err: fmt.Errorf("the secret of %s does not open", d.Name()),
		}
	}
	if !utf8.Valid(plain) {
		return provider.Revision{}, false, &provider.PoolError{
			Err: fmt.Errorf("the secret of %s is not UTF-8", d.Name()),
		}
	}
	return provider.Revision{Text: string(plain), Version: row.Version}, true, nil
}

// Replace commits a sealed secret only when its version still matches.
func (d *vaultDocument) Replace(ctx context.Context, text string, version uint64) (bool, error) {
	id, err := webapi.ParseCredentialID(d.id)
	if err != nil {
		return false, nil
	}
	sealed, err := d.pool.vault.SealSecret(d.pool.id, id, text)
	if err != nil {
		return false, &provider.PoolError{Err: err}
	}
	ctx = context.WithoutCancel(ctx)
	ok, err := d.pool.vault.control.ReplaceCredentialSecret(ctx, d.pool.id, id, sealed, version)
	if err != nil {
		return false, &provider.PoolError{Err: err}
	}
	if ok {
		d.pool.vault.MarkEntryChanged(ctx, d.pool.id)
	}
	return ok, nil
}

// RefreshTurn serializes refreshes of the account’s secret.
func (d *vaultDocument) RefreshTurn(ctx context.Context) (*gates.Permit, error) {
	id, err := webapi.ParseCredentialID(d.id)
	if err != nil {
		id = ""
	}
	key := string(d.pool.id) + "/" + string(id)
	return d.pool.vault.gates.Turn(ctx, key)
}
