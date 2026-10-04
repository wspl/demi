package database

import (
	"context"
	"database/sql"
	"errors"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// CreateDevice stores a device the user paired, with the hash of the token its
// runner receives.
func (c *ControlService) CreateDevice(
	ctx context.Context,
	user webapiproto.UserID,
	name string,
	platform runnerproto.RunnerPlatform,
	token TokenHash,
) (DeviceRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (DeviceRecord, error) {
		at, err := now.Millisecond()
		if err != nil {
			return DeviceRecord{}, err
		}
		d := DeviceRecord{
			ID:        webapiproto.DeviceID(uuid.NewString()),
			User:      user,
			Kind:      webapiproto.DeviceKindUser,
			Name:      name,
			Platform:  platform,
			ClaimedAt: now,
		}
		return d, execSQL(
			ctx,
			tx,
			`INSERT INTO devices (id,user_id,kind,name,platform,token_hash,claimed_at,last_seen_at)
VALUES (?,?,'user',?,?,?,?,NULL)`,
			d.ID,
			user,
			name,
			platform,
			token.Text(),
			at,
		)
	})
}

// Device returns the device with id, or nil when absent.
func (c *ControlService) Device(ctx context.Context, id webapiproto.DeviceID) (DeviceRecord, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (DeviceRecord, error) {
		r, ok, err := queryRecord(ctx, tx, "devices", "SELECT * FROM devices WHERE id = ?", deviceRow, id)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// DeviceByToken returns the device whose current token has this hash.
func (c *ControlService) DeviceByToken(ctx context.Context, token TokenHash) (DeviceRecord, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (DeviceRecord, error) {
		r, ok, err := queryRecord(
			ctx,
			tx,
			"devices",
			"SELECT * FROM devices WHERE token_hash = ?",
			deviceRow,
			token.Text(),
		)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// ManagedDevice returns the user's Cloud device, when its first use made it.
func (c *ControlService) ManagedDevice(ctx context.Context, user webapiproto.UserID) (DeviceRecord, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (DeviceRecord, error) {
		r, ok, err := queryRecord(
			ctx,
			tx,
			"devices",
			"SELECT * FROM devices WHERE kind = 'managed' AND user_id = ?",
			deviceRow,
			user,
		)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// ManagedDeviceOrCreate returns the user's Cloud device, made on its first use. The partial unique
// index admits one per user, so concurrent first uses find the same
// one. Its token is issued when it boots.
func (c *ControlService) ManagedDeviceOrCreate(ctx context.Context, user webapiproto.UserID) (DeviceRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (DeviceRecord, error) {
		at, err := now.Millisecond()
		if err != nil {
			return DeviceRecord{}, err
		}
		if err := execSQL(
			ctx,
			tx,
			`INSERT INTO devices (id,user_id,kind,name,platform,token_hash,claimed_at,last_seen_at)
VALUES (?,?,'managed','Cloud','linux',NULL,?,NULL)
ON CONFLICT DO NOTHING`,
			uuid.NewString(),
			user,
			at,
		); err != nil {
			return DeviceRecord{}, err
		}
		d, found, err := queryRecord(
			ctx,
			tx,
			"devices",
			"SELECT * FROM devices WHERE kind = 'managed' AND user_id = ?",
			deviceRow,
			user,
		)
		if err != nil {
			return DeviceRecord{}, err
		}
		if !found {
			return DeviceRecord{}, CorruptValue(
				"devices",
				"kind",
				errors.New("the user's Cloud device was neither found nor made"),
			)
		}
		return d, nil
	})
}

// PairedDevices returns the devices the user paired, oldest first.
func (c *ControlService) PairedDevices(ctx context.Context, user webapiproto.UserID) ([]DeviceRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]DeviceRecord, error) {
		return queryRecords(
			ctx,
			tx,
			"devices",
			"SELECT * FROM devices WHERE user_id = ? AND kind = 'user' ORDER BY claimed_at,id",
			deviceRow,
			user,
		)
	})
}

// WorkspacesOnDevice how many workspaces point at the device.
func (c *ControlService) WorkspacesOnDevice(ctx context.Context, device webapiproto.DeviceID) (uint64, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (uint64, error) {
		var count uint64
		err := tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM workspaces WHERE device_id = ?", device).Scan(&count)
		return count, err
	})
}

// DeleteDevice deletes the device with its attachments to conversations; its
// exposes go with it.
func (c *ControlService) DeleteDevice(ctx context.Context, device webapiproto.DeviceID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		if err := execSQL(ctx, tx, "DELETE FROM conversation_hosts WHERE device_id = ?", device); err != nil {
			return err
		}
		return execSQL(ctx, tx, "DELETE FROM devices WHERE id = ?", device)
	})
}

// TouchDeviceSeen records that the device's runner was connected just now.
func (c *ControlService) TouchDeviceSeen(ctx context.Context, device webapiproto.DeviceID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) error {
		at, err := now.Millisecond()
		if err != nil {
			return err
		}
		return execSQL(ctx, tx, "UPDATE devices SET last_seen_at = ? WHERE id = ?", at, device)
	})
}

func deviceRow(r *storedRow) DeviceRecord {
	d := DeviceRecord{
		ID:         checked(r, "id", webapiproto.ParseDeviceID),
		User:       checked(r, "user_id", webapiproto.ParseUserID),
		Kind:       webapiproto.DeviceKind(r.text("kind")),
		Name:       r.text("name"),
		Platform:   runnerproto.RunnerPlatform(r.text("platform")),
		ClaimedAt:  r.instant("claimed_at"),
		LastSeenAt: r.optionalInstant("last_seen_at"),
	}
	r.bad("kind", d.Kind.Validate())
	r.bad("platform", d.Platform.Validate())
	return d
}
