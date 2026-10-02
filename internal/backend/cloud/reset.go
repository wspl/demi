package cloud

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

// Reset admits id or returns the operation of that id: the same operation while
// running or ready, and a failed one resumed on its selected base. A new reset
// selects the current base. Another running reset is refused, as is a stopped
// Cloud with no capacity permit. Admitted work is owned and joined by the Cloud.
func Reset(ctx context.Context, shard CloudShard, id webapi.OperationID) (database.ManagedOperation, error) {
	panic("not written: b-cloud")
}

// RecoverResets runs before serving: reconcile stops and saves every machine
// and recovers incomplete operations; stale Cloud exposes are removed; each
// unfinished reset completes its idempotent disk step, is announced and is
// recorded as failed so a retry boots the Cloud. Nothing boots here.
func RecoverResets(ctx context.Context, control *database.ControlService, services *Services) error {
	panic("not written: b-cloud")
}
