package expose

import (
	"context"
	"time"

	"github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Store is the control storage used by exposes. ControlService implements it.
type Store interface {
	Device(context.Context, webapi.DeviceID) (database.DeviceRecord, bool, error)
	CreateExpose(
		context.Context,
		webapi.ExposeID,
		webapi.UserID,
		webapi.DeviceID,
		webapi.ExposeAddress,
		time.Duration,
	) (database.ExposeRecord, error)
	Expose(context.Context, webapi.ExposeID) (database.ExposeRecord, bool, error)
	UserExposes(context.Context, webapi.UserID) (database.UserExposes, error)
	RenewExpose(context.Context, webapi.ExposeID, webapi.UserID, time.Duration) (database.ExposeRecord, error)
	DeleteExpose(context.Context, webapi.ExposeID) error
	DeleteExpiredExpose(context.Context, webapi.ExposeID, core.Timestamp) (bool, error)
	DeleteDeviceExposes(context.Context, webapi.DeviceID) ([]webapi.ExposeID, error)
}

var _ Store = (*database.ControlService)(nil)

// Shard provides an expose's owner, configuration and synchronized shard operations.
// Configuration remains immutable for the shard's lifetime. Callbacks must be
// safe for concurrent calls; no caller holds a shard mutex across these operations.
type Shard interface {
	User() webapi.UserID
	Control() Store
	Clock() core.Clock
	ExposesChanged()
	Exposes() *Connections
	Domain() *Domain
	PublicURL() *url.Url
	DeviceConnected(database.DeviceRecord) bool
}
