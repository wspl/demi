package expose_test

import (
	"context"
	"sync/atomic"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

type expiryClock struct{ millis atomic.Int64 }

func (c *expiryClock) Now() types.Timestamp {
	at, err := types.TimestampFromTime(time.UnixMilli(c.millis.Load()))
	if err != nil {
		panic(err)
	}
	return at
}

type storedShard struct {
	*shard
	control expose.Store
	clock   types.Clock
	user    webapiproto.UserID
}

func (s *storedShard) Control() expose.Store    { return s.control }
func (s *storedShard) Clock() types.Clock       { return s.clock }
func (s *storedShard) User() webapiproto.UserID { return s.user }

type heldExposeRead struct {
	expose.Store
	arrived chan struct{}
	resume  chan struct{}
}

func (h *heldExposeRead) Expose(ctx context.Context, id webapiproto.ExposeID) (database.ExposeRecord, bool, error) {
	record, found, err := h.Store.Expose(ctx, id)
	close(h.arrived)
	select {
	case <-h.resume:
		return record, found, err
	case <-ctx.Done():
		return database.ExposeRecord{}, false, ctx.Err()
	}
}

// A real SQLite row is renewed after a relay reads its old expiry. The clock
// crosses that old expiry before the relay uses it. No sleeps or network IO.
func TestExpiredSnapshotCannotDeleteRenewal(t *testing.T) {
	clock := &expiryClock{}
	clock.millis.Store(1800000000000)
	control := databasetest.Control(t.Context(), t, clock)
	user := databasetest.Master(t.Context(), t, control)
	device, err := control.ManagedDeviceOrCreate(t.Context(), user.ID)
	if err != nil {
		t.Fatal(err)
	}
	s := &storedShard{shard: newShard(t), control: control, clock: clock, user: user.ID}
	value, err := expose.Add(t.Context(), s, device.ID, "80", time.Hour)
	if err != nil {
		t.Fatal(err)
	}
	existing, err := expose.AdmitRelay(t.Context(), s, value.Record.ID)
	if err != nil {
		t.Fatal(err)
	}
	defer existing.Release()
	held := &heldExposeRead{Store: control, arrived: make(chan struct{}), resume: make(chan struct{})}
	delayed := &storedShard{shard: s.shard, control: held, clock: clock, user: user.ID}
	done := make(chan error, 1)
	go func() {
		admission, err := expose.AdmitRelay(t.Context(), delayed, value.Record.ID)
		if admission != nil {
			admission.Release()
		}
		done <- err
	}()
	<-held.arrived
	clock.millis.Add(int64(30 * time.Minute / time.Millisecond))
	renewed, renewErr := expose.Renew(t.Context(), s, value.Record.ID, 2*time.Hour)
	clock.millis.Add(int64(31 * time.Minute / time.Millisecond))
	close(held.resume)
	admissionErr := <-done
	if renewErr != nil {
		t.Fatal(renewErr)
	}
	if admissionErr != nil {
		t.Errorf("relay refused renewed expose: %v", admissionErr)
	}
	record, found, err := control.Expose(t.Context(), value.Record.ID)
	if err != nil {
		t.Fatal(err)
	}
	if !found || record.ExpiresAt != renewed.Record.ExpiresAt {
		t.Fatalf("renewal lost: %#v", record)
	}
	select {
	case <-existing.Ending():
		t.Fatal("stale expiry ended an existing relay")
	default:
	}
}
