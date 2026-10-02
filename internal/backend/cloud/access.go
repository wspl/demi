package cloud

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
)

// MachineAccess is the user's Cloud, running, and a Host on it that starts in
// its home. The Cloud stays admitted until the owner calls Release.
type MachineAccess struct {
	// Device is the user's Cloud device record.
	Device database.DeviceRecord
	// Host starts in the Cloud's home and takes admission per operation.
	Host *remotehost.Host
	// Home is the home directory the Cloud's runner reported.
	Home string
}

// Release lets the Cloud admission go. It is idempotent and does not wait.
func (a *MachineAccess) Release() { panic("not written: b-cloud") }

// Device returns the user's Cloud device, made on its first use, which the
// user's pages then show among the devices and as the Cloud.
func Device(ctx context.Context, shard CloudShard) (database.DeviceRecord, error) {
	panic("not written: b-cloud")
}

// Access makes the user's Cloud on first use and wakes it when stopped. It
// takes Cloud admission and no conversation file gate; it is for machine work
// such as project creation and provider placement. Conversation files must use
// conversation host access. The caller defers the returned access's Release.
func Access(ctx context.Context, shard CloudShard) (*MachineAccess, error) {
	panic("not written: b-cloud")
}
