package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/webapi"
)

// CreateExpose returns a new expose `id` of `user` on `device`, from now until `lifetime`
// from now.
func (c *ControlService) CreateExpose(ctx context.Context, id webapi.ExposeID, user webapi.UserID, device webapi.DeviceID, address webapi.ExposeAddress, lifetime time.Duration) (ExposeRecord, error) {
	panic("not written: b-database")
}

// Expose returns the expose `id`, expired or not.
func (c *ControlService) Expose(ctx context.Context, id webapi.ExposeID) (*ExposeRecord, error) {
	panic("not written: b-database")
}

// UserExposes returns the exposes of `user`, after deleting the expired ones. A listing
// that finds none expired writes nothing.
func (c *ControlService) UserExposes(ctx context.Context, user webapi.UserID) (UserExposes, error) {
	panic("not written: b-database")
}

// RenewExpose moves the expiry of the live expose `id` of `user` to `lifetime` from
// now; none when `user` has no such live expose.
func (c *ControlService) RenewExpose(ctx context.Context, id webapi.ExposeID, user webapi.UserID, lifetime time.Duration) (*ExposeRecord, error) {
	panic("not written: b-database")
}

// DeleteExpose deletes the expose `id`, if there is one.
func (c *ControlService) DeleteExpose(ctx context.Context, id webapi.ExposeID) error {
	panic("not written: b-database")
}

// DeleteDeviceExposes deletes every expose on `device`; answers their ids.
func (c *ControlService) DeleteDeviceExposes(ctx context.Context, device webapi.DeviceID) ([]webapi.ExposeID, error) {
	panic("not written: b-database")
}

// DeleteCloudExposes deletes the exposes of every user's Cloud, as a backend that starts
// does: the machine manager has stopped every Cloud by then.
func (c *ControlService) DeleteCloudExposes(ctx context.Context) error {
	panic("not written: b-database")
}
