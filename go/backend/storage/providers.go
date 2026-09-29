package storage

import (
	"context"
	"database/sql"
	"encoding/json/v2"
	"errors"
	"math"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type NewProvider struct {
	ID     webapi.ProviderID
	Owner  webapi.UserID
	Family string
	Kind   webapi.CredentialKind
	Label  string
	Config []byte
	Active *webapi.CredentialID
}
type ProviderRow struct {
	NewProvider
	CreatedAt core.Timestamp
}
type CredentialWrite struct {
	ID          webapi.CredentialID
	IdentityKey *string
	Label       string
	Detail      *string
	Source      string
	Secret      []byte
}
type CredentialRow struct {
	CredentialWrite
	Version   uint64
	Quota     *core.QuotaSnapshot
	UpdatedAt core.Timestamp
}

//demi:wire
type CatalogRecord struct {
	Key       string                 `json:"key" check:"chars=1.."`
	CheckedAt core.Timestamp         `json:"checkedAt" check:"func=core.Validate"`
	Catalog   core.ProviderModelList `json:"catalog" check:"func=core.Validate"`
}

const providerColumns = "id,owner_user_id,provider_type,credential_kind,label,config,active_credential_id,created_at"
const credentialColumns = "id,identity_key,label,detail,source,secret,version,quota,updated_at"

func scanProvider(row scanner) (*ProviderRow, error) {
	var r ProviderRow
	var id, owner, kind string
	var active *string
	var ms int64
	err := row.Scan(&id, &owner, &r.Family, &kind, &r.Label, &r.Config, &active, &ms)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.ID, err = webapi.ParseProviderID(id)
	if err != nil {
		return nil, corrupt("providers", "id", err)
	}
	r.Owner, err = webapi.ParseUserID(owner)
	if err != nil {
		return nil, corrupt("providers", "owner_user_id", err)
	}
	r.Kind = webapi.CredentialKind(kind)
	if kind != "api_key" && kind != "subscription" {
		return nil, &CorruptError{"providers", "credential_kind", "unknown credential kind " + kind}
	}
	if active != nil {
		value, err := webapi.ParseCredentialID(*active)
		if err != nil {
			return nil, corrupt("providers", "active_credential_id", err)
		}
		r.Active = &value
	}
	r.CreatedAt, err = instant("providers", "created_at", ms)
	return &r, err
}
func scanCredential(row scanner) (*CredentialRow, error) {
	var r CredentialRow
	var id string
	var quota *string
	var ms int64
	err := row.Scan(&id, &r.IdentityKey, &r.Label, &r.Detail, &r.Source, &r.Secret, storedCount{"provider_credentials", "version", &r.Version}, &quota, &ms)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.ID, err = webapi.ParseCredentialID(id)
	if err != nil {
		return nil, corrupt("provider_credentials", "id", err)
	}
	if quota != nil {
		value, err := core.Decode[core.QuotaSnapshot]([]byte(*quota))
		if err != nil {
			return nil, corrupt("provider_credentials", "quota", err)
		}
		r.Quota = &value
	}
	r.UpdatedAt, err = instant("provider_credentials", "updated_at", ms)
	return &r, err
}
func (c *Control) Master(ctx context.Context) (*webapi.UserID, error) {
	var text string
	err := c.db.QueryRowContext(ctx, "SELECT id FROM users WHERE role='master'").Scan(&text)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	id, err := webapi.ParseUserID(text)
	return &id, corrupt("users", "id", err)
}
func writeCredential(ctx context.Context, db database, provider webapi.ProviderID, account CredentialWrite, now core.Timestamp) error {
	_, err := db.ExecContext(ctx, `INSERT INTO provider_credentials(provider_id,id,identity_key,label,detail,source,secret,version,updated_at) VALUES (?,?,?,?,?,?,?,1,?)
 ON CONFLICT(provider_id,id) DO UPDATE SET identity_key=excluded.identity_key,label=excluded.label,detail=excluded.detail,source=excluded.source,secret=excluded.secret,version=version+1,updated_at=excluded.updated_at`, provider.String(), account.ID.String(), account.IdentityKey, account.Label, account.Detail, account.Source, account.Secret, now.Millisecond())
	if err != nil {
		return sqliteError(err)
	}
	_, err = db.ExecContext(ctx, "UPDATE providers SET active_credential_id=? WHERE id=? AND active_credential_id IS NULL", account.ID.String(), provider.String())
	return sqliteError(err)
}
func (c *Control) InsertProvider(ctx context.Context, p NewProvider, accounts []CredentialWrite) (*ProviderRow, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	now := c.clock.Now()
	var active *string
	if p.Active != nil {
		text := p.Active.String()
		active = &text
	}
	r, err := scanProvider(tx.QueryRowContext(ctx, "INSERT INTO providers("+providerColumns+") VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(owner_user_id,provider_type) WHERE credential_kind='subscription' DO NOTHING RETURNING "+providerColumns, p.ID.String(), p.Owner.String(), p.Family, string(p.Kind), p.Label, p.Config, active, now.Millisecond()))
	if err != nil || r == nil {
		return nil, err
	}
	for _, account := range accounts {
		if err = writeCredential(ctx, tx, p.ID, account, now); err != nil {
			return nil, err
		}
	}
	return r, sqliteError(tx.Commit())
}
func (c *Control) Provider(ctx context.Context, id webapi.ProviderID) (*ProviderRow, error) {
	return scanProvider(c.db.QueryRowContext(ctx, "SELECT "+providerColumns+" FROM providers WHERE id=?", id.String()))
}
func (c *Control) Providers(ctx context.Context, owner webapi.UserID) ([]ProviderRow, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT "+providerColumns+" FROM providers WHERE owner_user_id=? ORDER BY created_at,rowid", owner.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	providers := []ProviderRow{}
	for rows.Next() {
		r, err := scanProvider(rows)
		if err != nil {
			return nil, err
		}
		providers = append(providers, *r)
	}
	return providers, sqliteError(rows.Err())
}
func (c *Control) UpdateProvider(ctx context.Context, id webapi.ProviderID, label *string, config []byte) (*ProviderRow, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	if label != nil {
		if _, err = tx.ExecContext(ctx, "UPDATE providers SET label=? WHERE id=?", *label, id.String()); err != nil {
			return nil, sqliteError(err)
		}
	}
	if config != nil {
		if _, err = tx.ExecContext(ctx, "UPDATE providers SET config=? WHERE id=?", config, id.String()); err != nil {
			return nil, sqliteError(err)
		}
	}
	r, err := scanProvider(tx.QueryRowContext(ctx, "SELECT "+providerColumns+" FROM providers WHERE id=?", id.String()))
	if err != nil {
		return nil, err
	}
	return r, sqliteError(tx.Commit())
}
func (c *Control) DeleteProvider(ctx context.Context, id webapi.ProviderID) error {
	_, err := c.db.ExecContext(ctx, "DELETE FROM providers WHERE id=?", id.String())
	return sqliteError(err)
}
func (c *Control) Credentials(ctx context.Context, provider webapi.ProviderID) ([]CredentialRow, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT "+credentialColumns+" FROM provider_credentials WHERE provider_id=? ORDER BY id", provider.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	credentials := []CredentialRow{}
	for rows.Next() {
		r, err := scanCredential(rows)
		if err != nil {
			return nil, err
		}
		credentials = append(credentials, *r)
	}
	return credentials, sqliteError(rows.Err())
}
func (c *Control) Credential(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID) (*CredentialRow, error) {
	return scanCredential(c.db.QueryRowContext(ctx, "SELECT "+credentialColumns+" FROM provider_credentials WHERE provider_id=? AND id=?", provider.String(), id.String()))
}
func (c *Control) WriteCredential(ctx context.Context, provider webapi.ProviderID, account CredentialWrite) (bool, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return false, sqliteError(err)
	}
	defer tx.Rollback()
	var exists bool
	if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM providers WHERE id=?)", provider.String()).Scan(&exists); err != nil {
		return false, sqliteError(err)
	}
	if !exists {
		return false, nil
	}
	if err = writeCredential(ctx, tx, provider, account, c.clock.Now()); err != nil {
		return false, err
	}
	return true, sqliteError(tx.Commit())
}
func (c *Control) ReplaceCredentialSecret(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID, secret []byte, version uint64) (bool, error) {
	if version > math.MaxInt64 {
		return false, nil
	}
	result, err := c.db.ExecContext(ctx, "UPDATE provider_credentials SET secret=?,version=version+1,updated_at=? WHERE provider_id=? AND id=? AND version=?", secret, c.clock.Now().Millisecond(), provider.String(), id.String(), version)
	if err != nil {
		return false, sqliteError(err)
	}
	count, err := result.RowsAffected()
	return count == 1, sqliteError(err)
}
func (c *Control) SetCredentialQuota(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID, quota core.QuotaSnapshot) error {
	document, err := json.Marshal(quota)
	if err != nil {
		return err
	}
	_, err = c.db.ExecContext(ctx, "UPDATE provider_credentials SET quota=? WHERE provider_id=? AND id=?", string(document), provider.String(), id.String())
	return sqliteError(err)
}
func (c *Control) RemoveCredential(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID) error {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return sqliteError(err)
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, "DELETE FROM provider_credentials WHERE provider_id=? AND id=?", provider.String(), id.String()); err != nil {
		return sqliteError(err)
	}
	if _, err = tx.ExecContext(ctx, "UPDATE providers SET active_credential_id=NULL WHERE id=? AND active_credential_id=?", provider.String(), id.String()); err != nil {
		return sqliteError(err)
	}
	return sqliteError(tx.Commit())
}
func (c *Control) SetActiveCredential(ctx context.Context, provider webapi.ProviderID, id webapi.CredentialID) (bool, error) {
	result, err := c.db.ExecContext(ctx, "UPDATE providers SET active_credential_id=?2 WHERE id=?1 AND EXISTS(SELECT 1 FROM provider_credentials WHERE provider_id=?1 AND id=?2)", provider.String(), id.String())
	if err != nil {
		return false, sqliteError(err)
	}
	count, err := result.RowsAffected()
	return count == 1, sqliteError(err)
}
func (c *Control) CatalogRecord(ctx context.Context, provider webapi.ProviderID) (*CatalogRecord, error) {
	var text string
	err := c.db.QueryRowContext(ctx, "SELECT record FROM model_catalogs WHERE provider_id=?", provider.String()).Scan(&text)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	var record CatalogRecord
	if err = json.Unmarshal([]byte(text), &record); err != nil {
		return nil, corrupt("model_catalogs", "record", err)
	}
	return &record, nil
}
func (c *Control) PutCatalogRecord(ctx context.Context, provider webapi.ProviderID, record CatalogRecord) error {
	document, err := json.Marshal(record)
	if err != nil {
		return err
	}
	_, err = c.db.ExecContext(ctx, "INSERT INTO model_catalogs(provider_id,record) SELECT ?1,?2 WHERE EXISTS(SELECT 1 FROM providers WHERE id=?1) ON CONFLICT(provider_id) DO UPDATE SET record=excluded.record", provider.String(), string(document))
	return sqliteError(err)
}
func (c *Control) DeleteCatalogRecord(ctx context.Context, provider webapi.ProviderID) error {
	_, err := c.db.ExecContext(ctx, "DELETE FROM model_catalogs WHERE provider_id=?", provider.String())
	return sqliteError(err)
}
