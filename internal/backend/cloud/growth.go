package cloud

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/machinemanagerproto"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// GrowVolume grows volume of the Cloud device to bytes, which must be positive
// and at most its quota. Only the shard owner's managed device can grow.
func GrowVolume(
	ctx context.Context,
	shard Shard,
	device webapiproto.DeviceID,
	volume runnerproto.VolumeName,
	bytes uint64,
) error {
	tuning := shard.CloudServices().Tuning
	quota := tuning.SystemQuota
	if volume == runnerproto.VolumeNameHome {
		quota = tuning.HomeQuota
	}
	if bytes == 0 || bytes > quota {
		return fmt.Errorf("%s volume quota exceeded", volume)
	}
	record, found, err := cloudRecords(shard).Device(ctx, device)
	if err != nil {
		return err
	}
	if !found || record.Kind != webapiproto.DeviceKindManaged || record.User != shard.User() {
		//nolint:staticcheck // Product text, shown to the user as it is.
		return errors.New("Only the Cloud grows its volumes")
	}
	_, err = Call(
		ctx,
		shard.CloudServices().Machines,
		machinemanagerproto.GrowVolumeParams{
			DeviceID: string(device),
			Volume:   machinemanagerproto.Volume(volume),
			Bytes:    bytes,
		},
	)
	return err
}
