package usershard

import (
	"context"
	"fmt"
	"time"

	"github.com/nlnwa/whatwg-url/url"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// exposeView resolves the method-name conflicts between the consumer interfaces.
type exposeView struct{ *Shard }

func (v exposeView) Control() expose.Store {
	return exposeStore{Store: v.services.Control, shard: v.Shard}
}
func (v exposeView) Exposes() *expose.Exposes { return &v.exposes }
func (v exposeView) Domain() *expose.Domain   { return v.services.ExposeDomain }
func (v exposeView) PublicURL() *url.Url {
	backend, ok := v.services.PublicURL.URL()
	if !ok {
		return nil
	}
	parsed, err := url.Parse(backend.String())
	if err != nil {
		return nil
	} // PublicURL already validated this URL at entry.
	return parsed
}
func (v exposeView) DeviceConnected(record database.DeviceRecord) bool {
	return v.devices.Online(record.ID) && (record.Kind != webapi.DeviceKindManaged || v.cloud.Runs(record.ID))
}
func (v exposeView) ExposesChanged() {
	v.plugins.Fire(plugin.TopicExposes, nil)
}

// exposeStore orders each device's create commit with Cloud-stop deletion.
// The permit spans IO, but the shard mutex never does. Checking connection
// again after admission prevents a pre-stop observation from admitting a row.
type exposeStore struct {
	expose.Store
	shard *Shard
}

func (s exposeStore) CreateExpose(ctx context.Context, id webapi.ExposeID, user webapi.UserID, device webapi.DeviceID, address webapi.ExposeAddress, lifetime time.Duration) (database.ExposeRecord, error) {
	permit, err := s.shard.deviceOrder.Acquire(ctx, device)
	if err != nil {
		return database.ExposeRecord{}, err
	}
	defer permit.Release()
	record, err := s.Device(ctx, device)
	if err != nil {
		return database.ExposeRecord{}, err
	}
	if record == nil || record.User != user {
		return database.ExposeRecord{}, expose.ErrDeviceNotFound
	}
	if !(exposeView{s.shard}).DeviceConnected(*record) {
		return database.ExposeRecord{}, &expose.DeviceOfflineError{Device: device}
	}
	return s.Store.CreateExpose(ctx, id, user, device, address, lifetime)
}

func (s *Shard) stopExposes(ctx context.Context, device webapi.DeviceID) error {
	permit, err := s.deviceOrder.Acquire(ctx, device)
	if err != nil {
		return err
	}
	defer permit.Release()
	ids, err := s.services.Control.DeleteDeviceExposes(ctx, device)
	if err != nil {
		return fmt.Errorf("the exposes of a device could not be destroyed: %w", err)
	}
	if len(ids) != 0 {
		(exposeView{s}).ExposesChanged()
	}
	s.exposes.End(ids)
	return nil
}

func exposeRecord(value expose.Expose) plugin.ExposeRecord {
	return plugin.ExposeRecord{ID: value.Record.ID, Device: value.Record.Device, Address: value.Record.Address, URL: value.URL, CreatedAt: value.Record.CreatedAt, ExpiresAt: value.Record.ExpiresAt}
}

var _ expose.ExposeShard = exposeView{}

// portExpose adds the current device name to the plugin's expose record.
func (s *Shard) portExpose(ctx context.Context, value expose.Expose) (plugin.ExposeRecord, error) {
	record := exposeRecord(value)
	record.DeviceName = string(value.Record.Device)
	device, err := s.services.Control.Device(ctx, value.Record.Device)
	if err != nil {
		return plugin.ExposeRecord{}, err
	}
	if device != nil {
		record.DeviceName = device.Name
	}
	return record, nil
}
