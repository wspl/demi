package database

import (
	"context"
	"database/sql"
	"errors"
	"math"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// Master returns the master account, which a shared instance's entries belong to.
func (c *ControlService) Master(ctx context.Context) (webapiproto.UserID, bool, error) {
	var found bool
	record, err := controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (webapiproto.UserID, error) {
			r, ok, err := queryRecord(
				ctx,
				tx,
				"users",
				"SELECT id FROM users WHERE role = 'master'",
				func(r *storedRow) webapiproto.UserID { return checked(r, "id", webapiproto.ParseUserID) },
			)
			found = ok
			return r, err
		},
	)
	return record, found && err == nil, err
}

// InsertProvider stores a new entry with its first accounts in one transaction.
// ErrSubscriptionExists when the owner already holds the family's subscription entry: then
// nothing is stored.
func (c *ControlService) InsertProvider(
	ctx context.Context,
	provider NewProvider,
	accounts []CredentialWrite,
) (ProviderRow, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (ProviderRow, error) {
		at, err := now.Millisecond()
		if err != nil {
			return ProviderRow{}, err
		}
		r, found, err := queryRecord(
			ctx,
			tx,
			"providers",
			`INSERT INTO providers (
    id,
    owner_user_id,
    provider_type,
    credential_kind,
    label,
    config,
    active_credential_id,
    created_at
)
VALUES (?,?,?,?,?,?,?,?)
ON CONFLICT (owner_user_id,provider_type)
WHERE credential_kind = 'subscription' DO NOTHING
RETURNING *`,
			providerRow,
			provider.ID,
			provider.Owner,
			provider.Family,
			provider.Kind,
			provider.Label,
			provider.Config,
			provider.Active,
			at,
		)
		if err != nil {
			return ProviderRow{}, err
		}
		if !found {
			return ProviderRow{}, ErrSubscriptionExists
		}
		for _, account := range accounts {
			if err := writeCredential(ctx, tx, r.ID, account, now); err != nil {
				return ProviderRow{}, err
			}
		}
		return r, nil
	})
}

// Provider returns the entry with id, or nil when absent.
func (c *ControlService) Provider(ctx context.Context, id webapiproto.ProviderID) (ProviderRow, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (ProviderRow, error) {
		r, ok, err := queryRecord(ctx, tx, "providers", "SELECT * FROM providers WHERE id = ?", providerRow, id)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// Providers returns the owner's entries, oldest first.
func (c *ControlService) Providers(ctx context.Context, owner webapiproto.UserID) ([]ProviderRow, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]ProviderRow, error) {
		return queryRecords(
			ctx,
			tx,
			"providers",
			"SELECT * FROM providers WHERE owner_user_id = ? ORDER BY created_at,rowid",
			providerRow,
			owner,
		)
	})
}

// UpdateProvider replaces the entry's label or sealed configuration, and answers the
// entry as it now is; ErrProviderNotFound for an entry that no longer exists.
func (c *ControlService) UpdateProvider(
	ctx context.Context,
	id webapiproto.ProviderID,
	label *string,
	config *[]byte,
) (ProviderRow, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (ProviderRow, error) {
		if label != nil {
			if err := execSQL(ctx, tx, "UPDATE providers SET label = ? WHERE id = ?", *label, id); err != nil {
				return ProviderRow{}, err
			}
		}
		if config != nil {
			if err := execSQL(ctx, tx, "UPDATE providers SET config = ? WHERE id = ?", *config, id); err != nil {
				return ProviderRow{}, err
			}
		}
		r, found, err := queryRecord(ctx, tx, "providers", "SELECT * FROM providers WHERE id = ?", providerRow, id)
		if err != nil {
			return ProviderRow{}, err
		}
		if !found {
			return ProviderRow{}, ErrProviderNotFound
		}
		return r, nil
	})
}

// DeleteProvider deletes the entry with its accounts and catalog record.
func (c *ControlService) DeleteProvider(ctx context.Context, id webapiproto.ProviderID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(ctx, tx, "DELETE FROM providers WHERE id = ?", id)
	})
}

// Credentials returns the entry's accounts, ordered by id.
func (c *ControlService) Credentials(ctx context.Context, provider webapiproto.ProviderID) ([]CredentialRow, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]CredentialRow, error) {
		return queryRecords(
			ctx,
			tx,
			"provider_credentials",
			"SELECT * FROM provider_credentials WHERE provider_id = ? ORDER BY id",
			credentialRow,
			provider,
		)
	})
}

// Credential returns the entry's account with id, or nil when absent.
func (c *ControlService) Credential(
	ctx context.Context,
	provider webapiproto.ProviderID,
	id webapiproto.CredentialID,
) (CredentialRow, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (CredentialRow, error) {
		r, ok, err := queryRecord(
			ctx,
			tx,
			"provider_credentials",
			"SELECT * FROM provider_credentials WHERE provider_id = ? AND id = ?",
			credentialRow,
			provider,
			id,
		)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// WriteCredential inserts the account, or replaces the one with its id and advances its
// version, and selects it when the entry has no active account, all in
// one transaction; `false` for an entry that no longer exists.
func (c *ControlService) WriteCredential(
	ctx context.Context,
	provider webapiproto.ProviderID,
	account CredentialWrite,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (bool, error) {
		var exists bool
		if err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM providers WHERE id = ?)", provider).
			Scan(&exists); err != nil {
			return false, err
		}
		if !exists {
			return false, nil
		}
		return true, writeCredential(ctx, tx, provider, account, now)
	})
}

// ReplaceCredentialSecret stores a refreshed secret only while the account is still at
// `version`; `false` when another writer stored first.
func (c *ControlService) ReplaceCredentialSecret(
	ctx context.Context,
	provider webapiproto.ProviderID,
	id webapiproto.CredentialID,
	secret []byte,
	version uint64,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (bool, error) {
		if version > math.MaxInt64 {
			return false, nil
		}
		at, err := now.Millisecond()
		if err != nil {
			return false, err
		}
		return affected(
			ctx,
			tx,
			`UPDATE provider_credentials
SET secret = ?,version = version + 1,updated_at = ?
WHERE provider_id = ? AND id = ? AND version = ?`,
			secret,
			at,
			provider,
			id,
			version,
		)
	})
}

// SetCredentialQuota replaces the account's kept quota snapshot; an account removed
// meanwhile keeps nothing.
func (c *ControlService) SetCredentialQuota(
	ctx context.Context,
	provider webapiproto.ProviderID,
	id webapiproto.CredentialID,
	quota types.QuotaSnapshot,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		text, err := encoded(quota)
		if err != nil {
			return err
		}
		return execSQL(
			ctx,
			tx,
			"UPDATE provider_credentials SET quota = ? WHERE provider_id = ? AND id = ?",
			text,
			provider,
			id,
		)
	})
}

// RemoveCredential removes the account, and the entry's selection of it with it.
func (c *ControlService) RemoveCredential(
	ctx context.Context,
	provider webapiproto.ProviderID,
	id webapiproto.CredentialID,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		if err := execSQL(
			ctx,
			tx,
			"DELETE FROM provider_credentials WHERE provider_id = ? AND id = ?",
			provider,
			id,
		); err != nil {
			return err
		}
		return execSQL(
			ctx,
			tx,
			"UPDATE providers SET active_credential_id = NULL WHERE id = ? AND active_credential_id = ?",
			provider,
			id,
		)
	})
}

// SetActiveCredential selects the account the entry infers with; `false` when the entry
// holds no such account.
func (c *ControlService) SetActiveCredential(
	ctx context.Context,
	provider webapiproto.ProviderID,
	id webapiproto.CredentialID,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (bool, error) {
		return affected(
			ctx,
			tx,
			`UPDATE providers
SET active_credential_id = ?2
WHERE id = ?1 AND EXISTS (SELECT 1 FROM provider_credentials WHERE provider_id = ?1 AND id = ?2)`,
			provider,
			id,
		)
	})
}

// CatalogRecord returns the entry's catalog record, validated.
func (c *ControlService) CatalogRecord(
	ctx context.Context,
	provider webapiproto.ProviderID,
) (CatalogRecord, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (CatalogRecord, error) {
		r, ok, err := queryRecord(
			ctx,
			tx,
			"model_catalogs",
			"SELECT record FROM model_catalogs WHERE provider_id = ?",
			func(r *storedRow) CatalogRecord { return storedJSON(r, "record", DecodeCatalogRecord) },
			provider,
		)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// PutCatalogRecord stores the entry's catalog record in place of the last one; an entry
// deleted meanwhile keeps nothing.
func (c *ControlService) PutCatalogRecord(
	ctx context.Context,
	provider webapiproto.ProviderID,
	record CatalogRecord,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		text, err := encoded(record)
		if err != nil {
			return err
		}
		return execSQL(
			ctx,
			tx,
			`INSERT INTO model_catalogs (provider_id,record)
SELECT ?1,?2
WHERE EXISTS (SELECT 1 FROM providers WHERE id = ?1)
ON CONFLICT (provider_id) DO UPDATE
SET record = excluded.record`,
			provider,
			text,
		)
	})
}

// DeleteCatalogRecord removes the entry's cached catalog.
func (c *ControlService) DeleteCatalogRecord(ctx context.Context, provider webapiproto.ProviderID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(ctx, tx, "DELETE FROM model_catalogs WHERE provider_id = ?", provider)
	})
}

func providerRow(r *storedRow) ProviderRow {
	p := ProviderRow{
		ID:        checked(r, "id", webapiproto.ParseProviderID),
		Owner:     checked(r, "owner_user_id", webapiproto.ParseUserID),
		Family:    r.text("provider_type"),
		Kind:      webapiproto.CredentialKind(r.text("credential_kind")),
		Label:     r.text("label"),
		Active:    optionalChecked(r, "active_credential_id", webapiproto.ParseCredentialID),
		CreatedAt: r.instant("created_at"),
	}
	if r.values["config"] != nil {
		v := r.bytes("config")
		p.Config = &v
	}
	r.bad("credential_kind", p.Kind.Validate())
	return p
}

func credentialRow(r *storedRow) CredentialRow {
	return CredentialRow{
		ID:          checked(r, "id", webapiproto.ParseCredentialID),
		IdentityKey: r.optionalText("identity_key"),
		Label:       r.text("label"),
		Detail:      r.optionalText("detail"),
		Source:      r.text("source"),
		Secret:      r.bytes("secret"),
		Version:     r.count("version"),
		Quota:       optionalJSON(r, "quota", types.DecodeQuotaSnapshot),
		UpdatedAt:   r.instant("updated_at"),
	}
}

func writeCredential(
	ctx context.Context,
	tx *sql.Tx,
	provider webapiproto.ProviderID,
	account CredentialWrite,
	now types.Timestamp,
) error {
	at, err := now.Millisecond()
	if err != nil {
		return err
	}
	if err := execSQL(
		ctx,
		tx,
		`INSERT INTO provider_credentials (provider_id,id,identity_key,label,detail,source,secret,version,updated_at)
VALUES (?,?,?,?,?,?,?,1,?)
ON CONFLICT (provider_id,id) DO UPDATE
SET
    identity_key=excluded.identity_key,
    label=excluded.label,
    detail=excluded.detail,
    source=excluded.source,
    secret=excluded.secret,
    version=version+1,
    updated_at=excluded.updated_at`,
		provider,
		account.ID,
		account.IdentityKey,
		account.Label,
		account.Detail,
		account.Source,
		account.Secret,
		at,
	); err != nil {
		return err
	}
	return execSQL(
		ctx,
		tx,
		"UPDATE providers SET active_credential_id = ? WHERE id = ? AND active_credential_id IS NULL",
		account.ID,
		provider,
	)
}

// ErrSubscriptionExists means the owner already holds the family's subscription entry; nothing is stored.
var ErrSubscriptionExists = errors.New("the owner already has this family's subscription entry")

// ErrProviderNotFound means no entry has the ID.
var ErrProviderNotFound = errors.New("no such provider entry")
