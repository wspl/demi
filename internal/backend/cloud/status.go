package cloud

import (
	"context"
	"log/slog"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/machineproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// Status reads the Cloud's device, lifecycle, latest reset, last error and disk
// capacities and limits without waking it. A manager read failure omits volumes.
func Status(ctx context.Context, shard Shard) (webapiproto.CloudStatus, error) {
	tuning := shard.CloudServices().Tuning
	status := webapiproto.CloudStatus{
		State:  webapiproto.CloudStateUnallocated,
		Limits: webapiproto.CloudVolumes{SystemBytes: tuning.SystemQuota, HomeBytes: tuning.HomeQuota},
	}
	device, found, err := cloudRecords(shard).ManagedDevice(ctx, shard.User())
	if err != nil || !found {
		return status, err
	}
	m, err := loadMachine(ctx, shard, device)
	if err != nil {
		return status, err
	}
	image, err := Call(
		ctx,
		shard.CloudServices().Machines,
		machineproto.ImageStateParams{DeviceID: string(device.ID)},
	)
	if err != nil {
		slog.Warn("the Cloud's disks could not be read", "device", device.ID, "error", err)
	} else if image != nil {
		status.Volumes = &webapiproto.CloudVolumes{SystemBytes: image.SystemBytes, HomeBytes: image.HomeBytes}
	}
	c := shard.Cloud()
	c.mu.Lock()
	status.Device = &webapiproto.CloudDevice{ID: device.ID, Name: device.Name}
	status.State = m.phase
	if m.operation != nil {
		dto := OperationDTO(*m.operation)
		status.Operation = &dto
	}
	status.Error = m.failure
	c.mu.Unlock()
	return status, nil
}

// OperationDTO returns a reset as the page sees it.
func OperationDTO(operation database.ManagedOperation) webapiproto.CloudOperation {
	return webapiproto.CloudOperation{ID: operation.ID, Phase: operation.Phase, Error: operation.Error}
}
