// Package machines is the machine manager's contract and its wire: the
// Provisioner every Cloud machine operation goes through, the manager's
// service end of the Unix socket (Serve), and the backend's end (Client).
//
// The wire carries newline-delimited JSON: one request per line, one ok or
// error reply per request, and death events on the manager's own initiative.
// Its types come from internal/contract/machineswire, generated from the Zod
// schemas in packages/machines/src/protocol.ts. See
// docs/demi-next/managed-hosts.md § Control and ownership.
package machines

import (
	"context"

	"github.com/wspl/demi/internal/contract/machineswire"
	"github.com/wspl/demi/internal/contract/runnerwire"
	"github.com/wspl/demi/internal/contract/zodrt"
)

// Volume names one of a device's two writable filesystems.
type Volume = machineswire.GrowVolumeParamsVolume

// The two writable filesystems of a device.
const (
	VolumeSystem = machineswire.GrowVolumeParamsVolumeSystem
	VolumeHome   = machineswire.GrowVolumeParamsVolumeHome
)

// Provisioner owns Cloud machines: their storage generations and at most one
// running sandbox per device. Operations on one device are serialized by the
// provisioner; disks never depend on a conversation.
type Provisioner interface {
	// Reconcile recovers incomplete operations and establishes saved,
	// stopped devices.
	Reconcile(ctx context.Context) error
	// CurrentBaseVersion reads the base selected for new devices and resets.
	CurrentBaseVersion(ctx context.Context) (string, error)
	// ImageState reads a device's committed generation, null when the device
	// has none.
	ImageState(ctx context.Context, deviceID string) (zodrt.Nullable[machineswire.MachineImageState], error)
	// RuntimeState reads whether the device has an active runtime, after
	// serializing with its transitions.
	RuntimeState(ctx context.Context, deviceID string) (machineswire.MachineRuntimeState, error)
	// Wake creates first-use storage or recovers existing storage, then
	// starts one sandbox with the boot credential.
	Wake(ctx context.Context, deviceID string, boot runnerwire.ManagedBoot) error
	// Hibernate stops execution, saves storage and releases the runtime.
	Hibernate(ctx context.Context, deviceID string) error
	// Checkpoint publishes paired system and home storage while the
	// sandbox's processes keep running.
	Checkpoint(ctx context.Context, deviceID string) error
	// GrowVolume increases one writable filesystem's capacity.
	GrowVolume(ctx context.Context, deviceID string, volume Volume, bytes int64) error
	// Reset publishes a clean system paired with the retained home,
	// idempotently by operation id. It does not boot.
	Reset(ctx context.Context, deviceID, operationID, baseVersion string) error
	// Close saves every machine and releases the provisioner's resources.
	Close(ctx context.Context) error
	// OnDeath registers a listener for unexpected runtime loss and returns
	// the function that removes it.
	OnDeath(listener func(deviceID string)) (remove func())
}
