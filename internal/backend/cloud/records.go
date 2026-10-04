package cloud

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapiproto"
)

// records is the control storage read and written by Cloud transitions. Keeping
// this boundary narrow lets timer tests replace IO with synchronized records.
type records interface {
	ManagedDevice(context.Context, webapiproto.UserID) (database.DeviceRecord, bool, error)
	ManagedDeviceOrCreate(context.Context, webapiproto.UserID) (database.DeviceRecord, error)
	LatestManagedOperation(context.Context, webapiproto.DeviceID) (database.ManagedOperation, bool, error)
	ManagedOperation(
		context.Context,
		webapiproto.DeviceID,
		webapiproto.OperationID,
	) (database.ManagedOperation, bool, error)
	PutManagedOperation(context.Context, webapiproto.DeviceID, database.ManagedOperation) error
	RotateDeviceToken(context.Context, webapiproto.DeviceID, database.TokenHash) error
	AnnounceCloudReset(context.Context, webapiproto.UserID, webapiproto.OperationID) error
	CloudUses(context.Context, webapiproto.UserID, *webapiproto.DeviceID) ([]database.CloudUseRecord, error)
	Device(context.Context, webapiproto.DeviceID) (database.DeviceRecord, bool, error)
}

// cloudRecords selects the Cloud's storage boundary; production uses the shard's control service.
func cloudRecords(s Shard) records {
	if s.Cloud().records != nil {
		return s.Cloud().records
	}
	return s.Control()
}
