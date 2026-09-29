package storage

import (
	"context"
	"database/sql"
	"errors"
	"math"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type ExposeRecord struct {
	ID                   webapi.ExposeID
	Number               uint64
	User                 webapi.UserID
	Device               webapi.DeviceID
	Address              webapi.ExposeAddress
	CreatedAt, ExpiresAt core.Timestamp
}
type UserExposes struct {
	Live    []ExposeRecord
	Expired []webapi.ExposeID
}

const exposeColumns = "id,number,user_id,device_id,address,created_at,expires_at"

func scanExpose(row scanner) (*ExposeRecord, error) {
	var r ExposeRecord
	var id, user, device, address string
	var created, expires int64
	err := row.Scan(&id, storedCount{"exposes", "number", &r.Number}, &user, &device, &address, &created, &expires)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.ID, err = webapi.ParseExposeID(id)
	if err != nil {
		return nil, corrupt("exposes", "id", err)
	}
	r.User, err = webapi.ParseUserID(user)
	if err != nil {
		return nil, corrupt("exposes", "user_id", err)
	}
	r.Device, err = webapi.ParseDeviceID(device)
	if err != nil {
		return nil, corrupt("exposes", "device_id", err)
	}
	r.Address, err = webapi.ParseExposeAddress(address)
	if err != nil {
		return nil, corrupt("exposes", "address", err)
	}
	r.CreatedAt, err = instant("exposes", "created_at", created)
	if err != nil {
		return nil, err
	}
	r.ExpiresAt, err = instant("exposes", "expires_at", expires)
	return &r, err
}
func (c *Control) CreateExpose(ctx context.Context, id webapi.ExposeID, user webapi.UserID, device webapi.DeviceID, address webapi.ExposeAddress, lifetime time.Duration) (*ExposeRecord, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	now := c.clock.Now()
	expiry, err := after(now, lifetime)
	if err != nil {
		return nil, err
	}
	var number uint64
	if err = tx.QueryRowContext(ctx, "UPDATE users SET next_expose=next_expose+1 WHERE id=? RETURNING next_expose-1", user.String()).Scan(&number); err != nil {
		return nil, sqliteError(err)
	}
	r, err := scanExpose(tx.QueryRowContext(ctx, "INSERT INTO exposes("+exposeColumns+") VALUES (?,?,?,?,?,?,?) RETURNING "+exposeColumns, id.String(), number, user.String(), device.String(), address.String(), now.Millisecond(), expiry.Millisecond()))
	if err != nil {
		return nil, err
	}
	return r, sqliteError(tx.Commit())
}
func (c *Control) Expose(ctx context.Context, id webapi.ExposeID) (*ExposeRecord, error) {
	return scanExpose(c.db.QueryRowContext(ctx, "SELECT "+exposeColumns+" FROM exposes WHERE id=?", id.String()))
}
func (c *Control) NumberedExpose(ctx context.Context, user webapi.UserID, number uint64) (*ExposeRecord, error) {
	return scanExpose(c.db.QueryRowContext(ctx, "SELECT "+exposeColumns+" FROM exposes WHERE user_id=? AND number=?", user.String(), int64(min(number, math.MaxInt64))))
}
func (c *Control) UserExposes(ctx context.Context, user webapi.UserID) (UserExposes, error) {
	answer := UserExposes{Live: []ExposeRecord{}, Expired: []webapi.ExposeID{}}
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return answer, sqliteError(err)
	}
	defer tx.Rollback()
	now := c.clock.Now().Millisecond()
	rows, err := tx.QueryContext(ctx, "SELECT "+exposeColumns+" FROM exposes WHERE user_id=? ORDER BY expires_at,id", user.String())
	if err != nil {
		return answer, sqliteError(err)
	}
	for rows.Next() {
		r, err := scanExpose(rows)
		if err != nil {
			rows.Close()
			return answer, err
		}
		if r.ExpiresAt.Millisecond() <= now {
			answer.Expired = append(answer.Expired, r.ID)
		} else {
			answer.Live = append(answer.Live, *r)
		}
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return answer, sqliteError(err)
	}
	for _, id := range answer.Expired {
		if _, err = tx.ExecContext(ctx, "DELETE FROM exposes WHERE id=?", id.String()); err != nil {
			return answer, sqliteError(err)
		}
	}
	return answer, sqliteError(tx.Commit())
}
func (c *Control) RenewExpose(ctx context.Context, id webapi.ExposeID, user webapi.UserID, lifetime time.Duration) (*ExposeRecord, error) {
	now := c.clock.Now()
	expiry, err := after(now, lifetime)
	if err != nil {
		return nil, err
	}
	return scanExpose(c.db.QueryRowContext(ctx, "UPDATE exposes SET expires_at=? WHERE id=? AND user_id=? AND expires_at>? RETURNING "+exposeColumns, expiry.Millisecond(), id.String(), user.String(), now.Millisecond()))
}
func (c *Control) DeleteExpose(ctx context.Context, id webapi.ExposeID) error {
	_, err := c.db.ExecContext(ctx, "DELETE FROM exposes WHERE id=?", id.String())
	return sqliteError(err)
}
func (c *Control) DeleteDeviceExposes(ctx context.Context, device webapi.DeviceID) ([]webapi.ExposeID, error) {
	rows, err := c.db.QueryContext(ctx, "DELETE FROM exposes WHERE device_id=? RETURNING id", device.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	ids := []webapi.ExposeID{}
	for rows.Next() {
		var text string
		if err = rows.Scan(&text); err != nil {
			return nil, sqliteError(err)
		}
		id, err := webapi.ParseExposeID(text)
		if err != nil {
			return nil, corrupt("exposes", "id", err)
		}
		ids = append(ids, id)
	}
	return ids, sqliteError(rows.Err())
}
func (c *Control) DeleteCloudExposes(ctx context.Context) error {
	_, err := c.db.ExecContext(ctx, "DELETE FROM exposes WHERE device_id IN (SELECT id FROM devices WHERE kind='managed')")
	return sqliteError(err)
}
