package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/webapi"
)

// ManagedOperation returns the device's reset `id`, as it was last written.
func (c *ControlService) ManagedOperation(ctx context.Context, device webapi.DeviceID, id webapi.OperationID) (*ManagedOperation, error) {
	panic("not written: b-database")
}

// LatestManagedOperation returns the device's reset written last: the one its status shows.
func (c *ControlService) LatestManagedOperation(ctx context.Context, device webapi.DeviceID) (*ManagedOperation, error) {
	panic("not written: b-database")
}

// PutManagedOperation writes the operation's phase and error, creating its row on its
// first write.
func (c *ControlService) PutManagedOperation(ctx context.Context, device webapi.DeviceID, operation ManagedOperation) error {
	panic("not written: b-database")
}

// UnfinishedManagedOperations returns every reset left between its admission and its end, with its
// device: what a backend that stopped in the middle of one finishes
// when it starts.
func (c *ControlService) UnfinishedManagedOperations(ctx context.Context) ([]DeviceOperation, error) {
	panic("not written: b-database")
}

// RotateDeviceToken replaces the token of the user's Cloud device with the one a boot
// minted: the token of an earlier boot opens no connection any more.
func (c *ControlService) RotateDeviceToken(ctx context.Context, device webapi.DeviceID, token TokenHash) error {
	panic("not written: b-database")
}

// AnnounceCloudReset tells every conversation of the user that its Cloud was reset by
// `operation`: each one's execution context advances once per reset,
// so every node reads the reset in its next context block.
func (c *ControlService) AnnounceCloudReset(ctx context.Context, user webapi.UserID, operation webapi.OperationID) error {
	panic("not written: b-database")
}

// AnnouncedCloudReset returns the Cloud reset the conversation was last told of.
func (c *ControlService) AnnouncedCloudReset(ctx context.Context, id webapi.ConversationID) (*webapi.OperationID, error) {
	panic("not written: b-database")
}

// CloudUses returns the user's conversations that are not archived, as the Cloud's
// lifecycle weighs them; `cloud` is the user's Cloud device, once its
// first use made it.
func (c *ControlService) CloudUses(ctx context.Context, user webapi.UserID, cloud *webapi.DeviceID) ([]CloudUseRecord, error) {
	panic("not written: b-database")
}
