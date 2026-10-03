package cloud

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// GrowVolume grows volume of the Cloud device to bytes, which must be positive
// and at most its quota. Only the shard owner's managed device can grow.
func GrowVolume(ctx context.Context, shard CloudShard, device webapi.DeviceID, volume runnerwire.VolumeName, bytes uint64) error {
	tuning := shard.CloudServices().Tuning
	quota := tuning.SystemQuota
	if volume == runnerwire.VolumeNameHome {
		quota = tuning.HomeQuota
	}
	if bytes == 0 || bytes > quota {
		return fmt.Errorf("%s volume quota exceeded", volume)
	}
	record, err := cloudRecords(shard).Device(ctx, device)
	if err != nil {
		return err
	}
	if record == nil || record.Kind != webapi.DeviceKindManaged || record.User != shard.User() {
		return errors.New("Only the Cloud grows its volumes") //nolint:staticcheck // Preserve Rust user-facing text verbatim.
	}
	_, err = Call(ctx, shard.CloudServices().Machines, machinewire.GrowVolumeParams{DeviceID: string(device), Volume: machinewire.Volume(volume), Bytes: bytes})
	return err
}
