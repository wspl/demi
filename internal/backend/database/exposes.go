package database

import (
	"context"
	"database/sql"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// CreateExpose returns a new expose `id` of `user` on `device`, from now until `lifetime`
// from now.
func (c *ControlService) CreateExpose(ctx context.Context, id webapi.ExposeID, user webapi.UserID, device webapi.DeviceID, address webapi.ExposeAddress, lifetime time.Duration) (ExposeRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) (ExposeRecord, error) {
		expiry, err := later(now, lifetime)
		if err != nil {
			return ExposeRecord{}, err
		}
		at, err := now.Millisecond()
		if err != nil {
			return ExposeRecord{}, err
		}
		end, err := expiry.Millisecond()
		if err != nil {
			return ExposeRecord{}, err
		}
		r := ExposeRecord{ID: id, User: user, Device: device, Address: address, CreatedAt: now, ExpiresAt: expiry}
		return r, execSQL(ctx, tx, "INSERT INTO exposes (id,user_id,device_id,address,created_at,expires_at) VALUES (?,?,?,?,?,?)", id, user, device, address, at, end)
	})
}

// Expose returns the expose `id`, expired or not.
func (c *ControlService) Expose(ctx context.Context, id webapi.ExposeID) (*ExposeRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*ExposeRecord, error) {
		return queryRecord(ctx, tx, "exposes", "SELECT * FROM exposes WHERE id = ?", exposeRow, id)
	})
}

// UserExposes returns the exposes of `user`, after deleting the expired ones. A listing
// that finds none expired writes nothing.
func (c *ControlService) UserExposes(ctx context.Context, user webapi.UserID) (UserExposes, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) (UserExposes, error) {
		rows, err := queryRecords(ctx, tx, "exposes", "SELECT * FROM exposes WHERE user_id = ? ORDER BY expires_at,id", exposeRow, user)
		if err != nil {
			return UserExposes{}, err
		}
		result := UserExposes{Live: []ExposeRecord{}, Expired: []webapi.ExposeID{}}
		for _, row := range rows {
			if row.ExpiresAt <= now {
				if err := execSQL(ctx, tx, "DELETE FROM exposes WHERE id = ?", row.ID); err != nil {
					return UserExposes{}, err
				}
				result.Expired = append(result.Expired, row.ID)
			} else {
				result.Live = append(result.Live, row)
			}
		}
		return result, nil
	})
}

// RenewExpose moves the expiry of the live expose `id` of `user` to `lifetime` from
// now; none when `user` has no such live expose.
func (c *ControlService) RenewExpose(ctx context.Context, id webapi.ExposeID, user webapi.UserID, lifetime time.Duration) (*ExposeRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) (*ExposeRecord, error) {
		expiry, err := later(now, lifetime)
		if err != nil {
			return nil, err
		}
		at, err := now.Millisecond()
		if err != nil {
			return nil, err
		}
		end, err := expiry.Millisecond()
		if err != nil {
			return nil, err
		}
		return queryRecord(ctx, tx, "exposes", "UPDATE exposes SET expires_at = ? WHERE id = ? AND user_id = ? AND expires_at > ? RETURNING *", exposeRow, end, id, user, at)
	})
}

// DeleteExpose deletes the expose `id`, if there is one.
func (c *ControlService) DeleteExpose(ctx context.Context, id webapi.ExposeID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return execSQL(ctx, tx, "DELETE FROM exposes WHERE id = ?", id)
	})
}

// DeleteExpiredExpose deletes id only if its current expiry is at or before
// observedAt. A renewal committed after the caller read the row survives.
// The result reports whether this transaction removed the row.
func (c *ControlService) DeleteExpiredExpose(ctx context.Context, id webapi.ExposeID, observedAt core.Timestamp) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (bool, error) {
		at, err := observedAt.Millisecond()
		if err != nil {
			return false, err
		}
		result, err := tx.ExecContext(ctx, "DELETE FROM exposes WHERE id = ? AND expires_at <= ?", id, at)
		if err != nil {
			return false, err
		}
		rows, err := result.RowsAffected()
		return rows != 0, err
	})
}

// DeleteDeviceExposes deletes every expose on `device`; answers their ids.
func (c *ControlService) DeleteDeviceExposes(ctx context.Context, device webapi.DeviceID) ([]webapi.ExposeID, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]webapi.ExposeID, error) {
		return queryRecords(ctx, tx, "exposes", "DELETE FROM exposes WHERE device_id = ? RETURNING id", func(r *storedRow) webapi.ExposeID { return checked(r, "id", webapi.ParseExposeID) }, device)
	})
}

// DeleteCloudExposes deletes the exposes of every user's Cloud, as a backend that starts
// does: the machine manager has stopped every Cloud by then.
func (c *ControlService) DeleteCloudExposes(ctx context.Context) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return execSQL(ctx, tx, "DELETE FROM exposes WHERE device_id IN (SELECT id FROM devices WHERE kind = 'managed')")
	})
}

func exposeRow(r *storedRow) ExposeRecord {
	return ExposeRecord{ID: checked(r, "id", webapi.ParseExposeID), User: checked(r, "user_id", webapi.ParseUserID), Device: checked(r, "device_id", webapi.ParseDeviceID), Address: checked(r, "address", webapi.ParseExposeAddress), CreatedAt: r.instant("created_at"), ExpiresAt: r.instant("expires_at")}
}
