package cloud

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

// Status reads the Cloud's device, lifecycle, latest reset, last error and disk
// capacities and limits without waking it. A manager read failure omits volumes.
func Status(ctx context.Context, shard CloudShard) (webapi.CloudStatus, error) {
	panic("not written: b-cloud")
}

// OperationDTO returns a reset as the page sees it.
func OperationDTO(operation database.ManagedOperation) webapi.CloudOperation {
	panic("not written: b-cloud")
}
