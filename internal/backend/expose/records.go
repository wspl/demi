package expose

import (
	"context"
	"crypto/rand"
	"encoding/base32"
	"errors"
	"fmt"
	"log/slog"
	"strings"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

// Expose is a live expose and its public URL.
type Expose struct {
	Record database.ExposeRecord
	URL    string
}

// Errors for expose operations.
var (
	// ErrUnavailable means this backend has no expose domain.
	//nolint:staticcheck // ST1005: user-visible text.
	ErrUnavailable = errors.New(
		"This backend has no expose domain configured (DEMI_EXPOSE_DOMAIN)",
	)
	// ErrDeviceNotFound means the device is not owned by the caller.
	//nolint:staticcheck // ST1005: user-visible text.
	ErrDeviceNotFound = errors.New(
		"No such device",
	)
	// ErrDeviceOffline means the runner is not connected or the Cloud is not running.
	ErrDeviceOffline = errors.New("is offline; connect it before exposing a service")
	// ErrNotFound means the caller has no live expose with this ID.
	//nolint:staticcheck // ST1005: user-visible text.
	ErrNotFound = errors.New("No expose")
)

// Add creates an expose on a connected device for lifetime.
func Add(
	ctx context.Context,
	shard ExposeShard,
	device webapi.DeviceID,
	address webapi.ExposeAddress,
	lifetime time.Duration,
) (Expose, error) {
	record, err := shard.Control().Device(ctx, device)
	if err != nil {
		return Expose{}, fmt.Errorf("read expose device: %w", err)
	}
	if record == nil || record.User != shard.User() {
		return Expose{}, ErrDeviceNotFound
	}
	domain := shard.Domain()
	if domain == nil {
		return Expose{}, ErrUnavailable
	}
	if !shard.DeviceConnected(*record) {
		//nolint:staticcheck // ST1005: user-visible text.
		return Expose{}, fmt.Errorf(
			"The device %s %w",
			device,
			ErrDeviceOffline,
		)
	}
	var bits [16]byte
	if _, err := rand.Read(bits[:]); err != nil {
		return Expose{}, fmt.Errorf("new expose id: %w", err)
	}
	id, err := webapi.ParseExposeID(
		strings.ToLower(base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(bits[:])),
	)
	if err != nil {
		return Expose{}, fmt.Errorf("new expose id: %w", err)
	}
	ctx = context.WithoutCancel(ctx)
	created, err := shard.Control().CreateExpose(ctx, id, shard.User(), device, address, lifetime)
	if err != nil {
		return Expose{}, fmt.Errorf("create expose: %w", err)
	}
	shard.ExposesChanged()
	return Expose{Record: created, URL: URL(created.ID, *domain, shard.PublicURL())}, nil
}

// List returns live exposes, soonest expiry first, and ends expired connections.
func List(ctx context.Context, shard ExposeShard) ([]Expose, error) {
	domain := shard.Domain()
	if domain == nil {
		return []Expose{}, nil
	}
	listed, err := shard.Control().UserExposes(context.WithoutCancel(ctx), shard.User())
	if err != nil {
		return nil, fmt.Errorf("list exposes: %w", err)
	}
	shard.Exposes().End(listed.Expired)
	result := make([]Expose, 0, len(listed.Live))
	for _, record := range listed.Live {
		result = append(result, Expose{Record: record, URL: URL(record.ID, *domain, shard.PublicURL())})
	}
	return result, nil
}

// Renew moves the expiry of an owned live expose to lifetime from now.
func Renew(ctx context.Context, shard ExposeShard, id webapi.ExposeID, lifetime time.Duration) (Expose, error) {
	domain := shard.Domain()
	if domain == nil {
		return Expose{}, fmt.Errorf("%w %s", ErrNotFound, id)
	}
	record, err := owned(ctx, shard, id)
	if err != nil {
		return Expose{}, err
	}
	if record == nil {
		return Expose{}, fmt.Errorf("%w %s", ErrNotFound, id)
	}
	expired, err := destroyIfExpired(ctx, shard, *record)
	if err != nil {
		return Expose{}, err
	}
	if expired {
		return Expose{}, fmt.Errorf("%w %s", ErrNotFound, id)
	}
	renewed, err := shard.Control().RenewExpose(context.WithoutCancel(ctx), id, shard.User(), lifetime)
	if err != nil {
		return Expose{}, fmt.Errorf("renew expose: %w", err)
	}
	if renewed == nil {
		return Expose{}, fmt.Errorf("%w %s", ErrNotFound, id)
	}
	shard.ExposesChanged()
	return Expose{Record: *renewed, URL: URL(renewed.ID, *domain, shard.PublicURL())}, nil
}

// Remove destroys an owned expose, answering not found if it already expired.
func Remove(ctx context.Context, shard ExposeShard, id webapi.ExposeID) error {
	if shard.Domain() == nil {
		return fmt.Errorf("%w %s", ErrNotFound, id)
	}
	record, err := owned(ctx, shard, id)
	if err != nil {
		return err
	}
	if record == nil {
		return fmt.Errorf("%w %s", ErrNotFound, id)
	}
	expired, err := destroyIfExpired(ctx, shard, *record)
	if err != nil {
		return err
	}
	if expired {
		return fmt.Errorf("%w %s", ErrNotFound, id)
	}
	return destroy(ctx, shard, id)
}

func owned(ctx context.Context, shard ExposeShard, id webapi.ExposeID) (*database.ExposeRecord, error) {
	record, err := shard.Control().Expose(ctx, id)
	if err != nil {
		return nil, fmt.Errorf("read expose: %w", err)
	}
	if record != nil && record.User != shard.User() {
		return nil, nil
	}
	return record, nil
}

func destroyIfExpired(ctx context.Context, shard ExposeShard, record database.ExposeRecord) (bool, error) {
	observedAt := shard.Clock().Now()
	now, err := observedAt.Time()
	if err != nil {
		return false, err
	}
	expiry, err := record.ExpiresAt.Time()
	if err != nil {
		return false, err
	}
	if now.Before(expiry) {
		return false, nil
	}
	deleted, err := shard.Control().DeleteExpiredExpose(context.WithoutCancel(ctx), record.ID, observedAt)
	if err != nil {
		return false, fmt.Errorf("delete expired expose: %w", err)
	}
	if deleted {
		shard.ExposesChanged()
		shard.Exposes().End([]webapi.ExposeID{record.ID})
	}
	return deleted, nil
}

func destroy(ctx context.Context, shard ExposeShard, id webapi.ExposeID) error {
	if err := shard.Control().DeleteExpose(context.WithoutCancel(ctx), id); err != nil {
		return fmt.Errorf("delete expose: %w", err)
	}
	shard.ExposesChanged()
	shard.Exposes().End([]webapi.ExposeID{id})
	return nil
}

// DestroyOn destroys the device's exposes during revocation or Cloud stop.
// A failure is logged: disconnecting the runner still ends its connections.
func DestroyOn(ctx context.Context, shard ExposeShard, device webapi.DeviceID) {
	ids, err := shard.Control().DeleteDeviceExposes(context.WithoutCancel(ctx), device)
	if err != nil {
		slog.ErrorContext(ctx, "the exposes of a device could not be destroyed: "+err.Error(), "device", device)
		return
	}
	if len(ids) != 0 {
		shard.ExposesChanged()
	}
	shard.Exposes().End(ids)
}
