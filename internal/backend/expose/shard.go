package expose

import (
	"context"
	"time"

	"github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// Store is the control storage used by exposes. ControlService implements it.
type Store interface {
	Device(context.Context, webapiproto.DeviceID) (database.DeviceRecord, bool, error)
	CreateExpose(
		context.Context,
		webapiproto.ExposeID,
		webapiproto.UserID,
		webapiproto.DeviceID,
		webapiproto.ExposeAddress,
		time.Duration,
	) (database.ExposeRecord, error)
	Expose(context.Context, webapiproto.ExposeID) (database.ExposeRecord, bool, error)
	UserExposes(context.Context, webapiproto.UserID) (database.UserExposes, error)
	RenewExpose(context.Context, webapiproto.ExposeID, webapiproto.UserID, time.Duration) (database.ExposeRecord, error)
	DeleteExpose(context.Context, webapiproto.ExposeID) error
	DeleteExpiredExpose(context.Context, webapiproto.ExposeID, types.Timestamp) (bool, error)
	DeleteDeviceExposes(context.Context, webapiproto.DeviceID) ([]webapiproto.ExposeID, error)
}

var _ Store = (*database.ControlService)(nil)

// Shard provides an expose's owner, configuration and synchronized shard operations.
// Configuration remains immutable for the shard's lifetime. Callbacks must be
// safe for concurrent calls; no caller holds a shard mutex across these operations.
type Shard interface {
	User() webapiproto.UserID
	Control() Store
	Clock() types.Clock
	ExposesChanged()
	Exposes() *Connections
	Domain() *Domain
	PublicURL() *url.Url
	DeviceConnected(database.DeviceRecord) bool
}
