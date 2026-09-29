package backend

import (
	"context"
	"fmt"

	"github.com/wspl/demi/go/machinesproto"
	"github.com/wspl/demi/go/webapi"
)

// resetRecovered is the failure a reset an earlier backend left unfinished
// ends with, once its disks are recovered.
const resetRecovered = "Reset disks recovered; retry to start Cloud"

// RecoverResets settles, before the backend serves, what an earlier backend
// left: the machine manager reconciles, which stops every Cloud and so ends
// their exposes, and each reset left unfinished commits its disks, is
// announced to its user and ends as failed (managed-hosts.md § Reset).
func RecoverResets(ctx context.Context, services *Services) error {
	if err := services.Machines.Reconcile(ctx); err != nil {
		return err
	}
	if err := services.Control.DeleteCloudExposes(ctx); err != nil {
		return err
	}
	unfinished, err := services.Control.UnfinishedManagedOperations(ctx)
	if err != nil {
		return err
	}
	for _, left := range unfinished {
		record, err := services.Control.Device(ctx, left.Device)
		if err != nil {
			return err
		}
		if record == nil {
			return fmt.Errorf("a reset names the device %s, which no longer exists", left.Device)
		}
		operation := left.Operation
		if err := services.Machines.Reset(ctx, machinesproto.DeviceID(left.Device.String()), operation.ID.String(), operation.BaseVersion); err != nil {
			return err
		}
		if err := services.Control.AnnounceCloudReset(ctx, record.User, operation.ID); err != nil {
			return err
		}
		operation.Phase = webapi.ResetPhaseFailed
		operation.Error = new(resetRecovered)
		if err := services.Control.PutManagedOperation(ctx, left.Device, operation); err != nil {
			return err
		}
	}
	return nil
}
