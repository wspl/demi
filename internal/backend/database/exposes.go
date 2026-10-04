package database

import (
	"context"
	"database/sql"
	"errors"
	"time"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// CreateExpose returns a new expose `id` of `user` on `device`, from now until `lifetime`
// from now.
func (c *ControlService) CreateExpose(
	ctx context.Context,
	id webapiproto.ExposeID,
	user webapiproto.UserID,
	device webapiproto.DeviceID,
	address webapiproto.ExposeAddress,
	lifetime time.Duration,
) (ExposeRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (ExposeRecord, error) {
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
		return r, execSQL(
			ctx,
			tx,
			"INSERT INTO exposes (id,user_id,device_id,address,created_at,expires_at) VALUES (?,?,?,?,?,?)",
			id,
			user,
			device,
			address,
			at,
			end,
		)
	})
}

// Expose returns the expose `id`, expired or not.
func (c *ControlService) Expose(ctx context.Context, id webapiproto.ExposeID) (ExposeRecord, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (ExposeRecord, error) {
		r, ok, err := queryRecord(ctx, tx, "exposes", "SELECT * FROM exposes WHERE id = ?", exposeRow, id)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// UserExposes returns the exposes of `user`, after deleting the expired ones. A listing
// that finds none expired writes nothing.
func (c *ControlService) UserExposes(ctx context.Context, user webapiproto.UserID) (UserExposes, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (UserExposes, error) {
		rows, err := queryRecords(
			ctx,
			tx,
			"exposes",
			"SELECT * FROM exposes WHERE user_id = ? ORDER BY expires_at,id",
			exposeRow,
			user,
		)
		if err != nil {
			return UserExposes{}, err
		}
		result := UserExposes{Live: []ExposeRecord{}, Expired: []webapiproto.ExposeID{}}
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
// now; ErrExposeNotFound when `user` has no such live expose.
func (c *ControlService) RenewExpose(
	ctx context.Context,
	id webapiproto.ExposeID,
	user webapiproto.UserID,
	lifetime time.Duration,
) (ExposeRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (ExposeRecord, error) {
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
		r, found, err := queryRecord(
			ctx,
			tx,
			"exposes",
			"UPDATE exposes SET expires_at = ? WHERE id = ? AND user_id = ? AND expires_at > ? RETURNING *",
			exposeRow,
			end,
			id,
			user,
			at,
		)
		if err != nil {
			return ExposeRecord{}, err
		}
		if !found {
			return ExposeRecord{}, ErrExposeNotFound
		}
		return r, nil
	})
}

// DeleteExpose deletes the expose `id`, if there is one.
func (c *ControlService) DeleteExpose(ctx context.Context, id webapiproto.ExposeID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(ctx, tx, "DELETE FROM exposes WHERE id = ?", id)
	})
}

// DeleteExpiredExpose deletes id only if its current expiry is at or before
// observedAt. A renewal committed after the caller read the row survives.
// The result reports whether this transaction removed the row.
func (c *ControlService) DeleteExpiredExpose(
	ctx context.Context,
	id webapiproto.ExposeID,
	observedAt types.Timestamp,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (bool, error) {
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
func (c *ControlService) DeleteDeviceExposes(
	ctx context.Context,
	device webapiproto.DeviceID,
) ([]webapiproto.ExposeID, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]webapiproto.ExposeID, error) {
			return queryRecords(
				ctx,
				tx,
				"exposes",
				"DELETE FROM exposes WHERE device_id = ? RETURNING id",
				func(r *storedRow) webapiproto.ExposeID {
					return checked(r, "id", webapiproto.ParseExposeID)
				},
				device,
			)
		},
	)
}

// DeleteCloudExposes deletes the exposes of every user's Cloud, as a backend that starts
// does: the machine manager has stopped every Cloud by then.
func (c *ControlService) DeleteCloudExposes(ctx context.Context) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(
			ctx,
			tx,
			"DELETE FROM exposes WHERE device_id IN (SELECT id FROM devices WHERE kind = 'managed')",
		)
	})
}

func exposeRow(r *storedRow) ExposeRecord {
	return ExposeRecord{
		ID:        checked(r, "id", webapiproto.ParseExposeID),
		User:      checked(r, "user_id", webapiproto.ParseUserID),
		Device:    checked(r, "device_id", webapiproto.ParseDeviceID),
		Address:   checked(r, "address", webapiproto.ParseExposeAddress),
		CreatedAt: r.instant("created_at"),
		ExpiresAt: r.instant("expires_at"),
	}
}

// ErrExposeNotFound means the user has no live expose of the ID.
var ErrExposeNotFound = errors.New("no live expose of the user has the id")
