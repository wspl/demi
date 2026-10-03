package cloud

import (
	"context"
	"errors"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
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
	Home      string
	admission *Admission
}

// Release lets the Cloud admission go. It is idempotent and does not wait.
func (a *MachineAccess) Release() { a.admission.Release() }

// Device returns the user's Cloud device, made on its first use, which the
// user's pages then show among the devices and as the Cloud.
func Device(ctx context.Context, shard CloudShard) (database.DeviceRecord, error) {
	control := cloudRecords(shard)
	device, found, err := control.ManagedDevice(ctx, shard.User())
	if err != nil {
		return database.DeviceRecord{}, err
	}
	if found {
		return device, nil
	}
	created, err := control.ManagedDeviceOrCreate(ctx, shard.User())
	if err != nil {
		return database.DeviceRecord{}, err
	}
	marks := shard.Marks()
	marks.Mark(pagesync.Part{Kind: pagesync.Devices})
	marks.Mark(pagesync.Part{Kind: pagesync.Cloud})
	return created, nil
}

// Access makes the user's Cloud on first use and wakes it when stopped. It
// takes Cloud admission and no conversation file gate; it is for machine work
// such as project creation and provider placement. Conversation files must use
// conversation host access. The caller defers the returned access's Release.
func Access(ctx context.Context, shard CloudShard) (*MachineAccess, error) {
	device, err := Device(ctx, shard)
	if err != nil {
		return nil, storageFailed(err)
	}
	admission, err := Admit(ctx, shard, device)
	if err != nil {
		return nil, err
	}
	home, ok := shard.Devices().Home(device.ID)
	if !ok {
		admission.Release()
		//nolint:staticcheck // Product text, shown to the user as it is.
		return nil, failed(errors.New("The Cloud did not report its home directory"))
	}
	return &MachineAccess{
		Device:    device,
		Host:      shard.Devices().MachineHost(device.ID, home, admission.PerOperation),
		Home:      home,
		admission: admission,
	}, nil
}
