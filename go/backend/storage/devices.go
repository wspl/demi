package storage

import (
	"context"
	"database/sql"
	"errors"

	"github.com/google/uuid"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/webapi"
)

type DeviceRecord struct {
	ID         webapi.DeviceID
	User       webapi.UserID
	Kind       webapi.DeviceKind
	Name       string
	Platform   runnerproto.RunnerPlatform
	ClaimedAt  core.Timestamp
	LastSeenAt *core.Timestamp
}

const deviceColumns = "id,user_id,kind,name,platform,claimed_at,last_seen_at"

func scanDevice(row scanner) (*DeviceRecord, error) {
	var r DeviceRecord
	var id, user, kind, platform string
	var claimed int64
	var seen *int64
	err := row.Scan(&id, &user, &kind, &r.Name, &platform, &claimed, &seen)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.ID, err = webapi.ParseDeviceID(id)
	if err != nil {
		return nil, corrupt("devices", "id", err)
	}
	r.User, err = webapi.ParseUserID(user)
	if err != nil {
		return nil, corrupt("devices", "user_id", err)
	}
	r.Kind = webapi.DeviceKind(kind)
	if kind != "user" && kind != "managed" {
		return nil, &CorruptError{"devices", "kind", "unknown kind: " + kind}
	}
	r.Platform, err = runnerproto.ParseRunnerPlatform(platform)
	if err != nil {
		return nil, corrupt("devices", "platform", err)
	}
	r.ClaimedAt, err = instant("devices", "claimed_at", claimed)
	if err != nil {
		return nil, err
	}
	if seen != nil {
		value, err := instant("devices", "last_seen_at", *seen)
		if err != nil {
			return nil, err
		}
		r.LastSeenAt = &value
	}
	return &r, nil
}
func (c *Control) CreateDevice(ctx context.Context, user webapi.UserID, name string, platform runnerproto.RunnerPlatform, token TokenHash) (*DeviceRecord, error) {
	return scanDevice(c.db.QueryRowContext(ctx, "INSERT INTO devices(id,user_id,kind,name,platform,token_hash,claimed_at,last_seen_at) VALUES (?,?,'user',?,?,?,?,NULL) RETURNING "+deviceColumns, uuid.NewString(), user.String(), name, string(platform), token.Text(), c.clock.Now().Millisecond()))
}
func (c *Control) Device(ctx context.Context, id webapi.DeviceID) (*DeviceRecord, error) {
	return scanDevice(c.db.QueryRowContext(ctx, "SELECT "+deviceColumns+" FROM devices WHERE id=?", id.String()))
}
func (c *Control) DeviceByToken(ctx context.Context, token TokenHash) (*DeviceRecord, error) {
	return scanDevice(c.db.QueryRowContext(ctx, "SELECT "+deviceColumns+" FROM devices WHERE token_hash=?", token.Text()))
}
func (c *Control) ManagedDevice(ctx context.Context, user webapi.UserID) (*DeviceRecord, error) {
	return scanDevice(c.db.QueryRowContext(ctx, "SELECT "+deviceColumns+" FROM devices WHERE kind='managed' AND user_id=?", user.String()))
}
func (c *Control) ManagedDeviceOrCreate(ctx context.Context, user webapi.UserID) (*DeviceRecord, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, "INSERT INTO devices(id,user_id,kind,name,platform,token_hash,claimed_at,last_seen_at) VALUES (?,?,'managed','Cloud','linux',NULL,?,NULL) ON CONFLICT DO NOTHING", uuid.NewString(), user.String(), c.clock.Now().Millisecond())
	if err != nil {
		return nil, sqliteError(err)
	}
	device, err := scanDevice(tx.QueryRowContext(ctx, "SELECT "+deviceColumns+" FROM devices WHERE kind='managed' AND user_id=?", user.String()))
	if err != nil {
		return nil, err
	}
	if device == nil {
		return nil, &CorruptError{"devices", "kind", "the user's Cloud device was neither found nor made"}
	}
	return device, sqliteError(tx.Commit())
}
func (c *Control) PairedDevices(ctx context.Context, user webapi.UserID) ([]DeviceRecord, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT "+deviceColumns+" FROM devices WHERE user_id=? AND kind='user' ORDER BY claimed_at,id", user.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	devices := []DeviceRecord{}
	for rows.Next() {
		r, err := scanDevice(rows)
		if err != nil {
			return nil, err
		}
		devices = append(devices, *r)
	}
	return devices, sqliteError(rows.Err())
}
func (c *Control) WorkspacesOnDevice(ctx context.Context, device webapi.DeviceID) (uint64, error) {
	var count uint64
	err := c.db.QueryRowContext(ctx, "SELECT COUNT(*) FROM workspaces WHERE device_id=?", device.String()).Scan(&count)
	return count, sqliteError(err)
}
func (c *Control) DeleteDevice(ctx context.Context, device webapi.DeviceID) error {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return sqliteError(err)
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, "DELETE FROM conversation_hosts WHERE device_id=?", device.String()); err != nil {
		return sqliteError(err)
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM devices WHERE id=?", device.String()); err != nil {
		return sqliteError(err)
	}
	return sqliteError(tx.Commit())
}
func (c *Control) TouchDeviceSeen(ctx context.Context, device webapi.DeviceID) error {
	_, err := c.db.ExecContext(ctx, "UPDATE devices SET last_seen_at=? WHERE id=?", c.clock.Now().Millisecond(), device.String())
	return sqliteError(err)
}
