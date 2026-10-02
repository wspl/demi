package cloud

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// GrowVolume grows volume of the Cloud device to bytes, which must be positive
// and at most its quota. Only the shard owner's managed device can grow.
func GrowVolume(ctx context.Context, shard CloudShard, device webapi.DeviceID, volume runnerwire.VolumeName, bytes uint64) error {
	panic("not written: b-cloud")
}
