package cloud

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

// records is the control storage read and written by Cloud transitions. Keeping
// this boundary narrow lets timer tests replace IO with synchronized records.
type records interface {
	ManagedDevice(context.Context, webapi.UserID) (database.DeviceRecord, bool, error)
	ManagedDeviceOrCreate(context.Context, webapi.UserID) (database.DeviceRecord, error)
	LatestManagedOperation(context.Context, webapi.DeviceID) (database.ManagedOperation, bool, error)
	ManagedOperation(context.Context, webapi.DeviceID, webapi.OperationID) (database.ManagedOperation, bool, error)
	PutManagedOperation(context.Context, webapi.DeviceID, database.ManagedOperation) error
	RotateDeviceToken(context.Context, webapi.DeviceID, database.TokenHash) error
	AnnounceCloudReset(context.Context, webapi.UserID, webapi.OperationID) error
	CloudUses(context.Context, webapi.UserID, *webapi.DeviceID) ([]database.CloudUseRecord, error)
	Device(context.Context, webapi.DeviceID) (database.DeviceRecord, bool, error)
}

// cloudRecords selects the Cloud's storage boundary; production uses the shard's control service.
func cloudRecords(s CloudShard) records {
	if s.Cloud().records != nil {
		return s.Cloud().records
	}
	return s.Control()
}
