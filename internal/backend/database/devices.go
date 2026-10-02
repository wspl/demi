package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// CreateDevice stores a device the user paired, with the hash of the token its
// runner receives.
func (c *ControlService) CreateDevice(ctx context.Context, user webapi.UserID, name string, platform runnerwire.RunnerPlatform, token TokenHash) (DeviceRecord, error) {
	panic("not written: b-database")
}

// Device returns the device with id, or nil when absent.
func (c *ControlService) Device(ctx context.Context, id webapi.DeviceID) (*DeviceRecord, error) {
	panic("not written: b-database")
}

// DeviceByToken returns the device whose current token has this hash.
func (c *ControlService) DeviceByToken(ctx context.Context, token TokenHash) (*DeviceRecord, error) {
	panic("not written: b-database")
}

// ManagedDevice returns the user's Cloud device, when its first use made it.
func (c *ControlService) ManagedDevice(ctx context.Context, user webapi.UserID) (*DeviceRecord, error) {
	panic("not written: b-database")
}

// ManagedDeviceOrCreate returns the user's Cloud device, made on its first use. The partial unique
// index admits one per user, so concurrent first uses find the same
// one. Its token is issued when it boots.
func (c *ControlService) ManagedDeviceOrCreate(ctx context.Context, user webapi.UserID) (DeviceRecord, error) {
	panic("not written: b-database")
}

// PairedDevices returns the devices the user paired, oldest first.
func (c *ControlService) PairedDevices(ctx context.Context, user webapi.UserID) ([]DeviceRecord, error) {
	panic("not written: b-database")
}

// WorkspacesOnDevice how many workspaces point at the device.
func (c *ControlService) WorkspacesOnDevice(ctx context.Context, device webapi.DeviceID) (uint64, error) {
	panic("not written: b-database")
}

// DeleteDevice deletes the device with its attachments to conversations; its
// exposes go with it.
func (c *ControlService) DeleteDevice(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-database")
}

// TouchDeviceSeen records that the device's runner was connected just now.
func (c *ControlService) TouchDeviceSeen(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-database")
}
