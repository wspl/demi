package machinewire

import (
	"encoding/json"

	"github.com/wspl/demi/internal/runnerwire"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// A request: an id the client chooses, which its reply names, and the call.
// +demi:root
// +demi:tolerant
type MachineRequest struct {
	// +demi:length chars min=1
	ID string `json:"id"`
	// +demi:flatten
	Call MachineCall
}

// An operation and its parameters. Keys a message does not declare are
// ignored. A device id or base version travels as a string: a name that
// breaks the image-name rule is the operation's error, not a malformed
// message, so the connection stays usable.
// +demi:root
// +demi:union tag=op content=params
//
//sumtype:decl
type MachineCall interface {
	machineCall()
	Name() string
}

// Stop and save every device, recover incomplete operations, and install
// the network policy again.
// +demi:root
// +demi:tolerant
type ReconcileParams struct{}

// +demi:variant MachineCall reconcile
// +demi:tolerant
type Reconcile struct {
	Params ReconcileParams `json:"params"`
}

func (*Reconcile) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*Reconcile) Name() string { return "reconcile" }

// Read the configured base.
// +demi:root
// +demi:tolerant
type CurrentBaseVersionParams struct{}

// +demi:variant MachineCall current_base_version
// +demi:tolerant
type CurrentBaseVersion struct {
	Params CurrentBaseVersionParams `json:"params"`
}

func (*CurrentBaseVersion) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*CurrentBaseVersion) Name() string { return "current_base_version" }

// Read a device's committed generation.
// +demi:root
// +demi:tolerant
type ImageStateParams struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
}

// +demi:variant MachineCall image_state
// +demi:tolerant
type ImageState struct {
	Params ImageStateParams `json:"params"`
}

func (*ImageState) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*ImageState) Name() string { return "image_state" }

// Read whether the manager runs a sandbox for a device, after the device's
// earlier operations.
// +demi:root
// +demi:tolerant
type RuntimeStateParams struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
}

// +demi:variant MachineCall runtime_state
// +demi:tolerant
type RuntimeStateCall struct {
	Params RuntimeStateParams `json:"params"`
}

func (*RuntimeStateCall) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*RuntimeStateCall) Name() string { return "runtime_state" }

// Create first-use storage or recover existing storage, then start one
// sandbox with the boot credential.
// +demi:root
// +demi:tolerant
type WakeParams struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
	// Checked as it is read: an unknown key refuses the message.
	Boot runnerwire.ManagedBoot `json:"boot"`
}

// +demi:variant MachineCall wake
// +demi:tolerant
type Wake struct {
	Params WakeParams `json:"params"`
}

func (*Wake) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*Wake) Name() string { return "wake" }

// Stop execution, save storage and release runtime resources.
// +demi:root
// +demi:tolerant
type HibernateParams struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
}

// +demi:variant MachineCall hibernate
// +demi:tolerant
type Hibernate struct {
	Params HibernateParams `json:"params"`
}

func (*Hibernate) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*Hibernate) Name() string { return "hibernate" }

// Publish the running device's storage while preserving its processes.
// +demi:root
// +demi:tolerant
type CheckpointParams struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
}

// +demi:variant MachineCall checkpoint
// +demi:tolerant
type Checkpoint struct {
	Params CheckpointParams `json:"params"`
}

func (*Checkpoint) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*Checkpoint) Name() string { return "checkpoint" }

// Grow one of the running device's filesystems to at least `bytes`.
// +demi:root
// +demi:tolerant
type GrowVolumeParams struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
	Volume   Volume `json:"volume"`
	// +demi:range min=1
	Bytes uint64 `json:"bytes"`
}

// +demi:variant MachineCall grow_volume
// +demi:tolerant
type GrowVolume struct {
	Params GrowVolumeParams `json:"params"`
}

func (*GrowVolume) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*GrowVolume) Name() string { return "grow_volume" }

// Publish a clean system on `base_version` with the retained home, once
// per `operation_id`; does not boot.
// +demi:root
// +demi:tolerant
type ResetParams struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
	// +demi:length chars min=1
	OperationID string `json:"operationId"`
	// +demi:length chars min=1
	BaseVersion string `json:"baseVersion"`
}

// +demi:variant MachineCall reset
// +demi:tolerant
type Reset struct {
	Params ResetParams `json:"params"`
}

func (*Reset) machineCall() {}

// Name returns the operation's name on the wire, such as grow_volume.
func (*Reset) Name() string { return "reset" }

// A message from the manager: the reply to a request, or the death of a
// device's sandbox, which every connection receives.
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type MachineResponse interface {
	machineResponse()
	WireMessage()
}

// The operation's result, which the client decodes as the operation's
// [`Operation::Output`] once it knows which request this answers.
// +demi:variant MachineResponse ok
// +demi:tolerant
type OK struct {
	// +demi:length chars min=1
	ID     string          `json:"id"`
	Result json.RawMessage `json:"result"`
}

func (*OK) machineResponse() {}

// WireMessage identifies a message of the machine-manager socket.
func (*OK) WireMessage() {}

// +demi:variant MachineResponse error
// +demi:tolerant
type ErrorResponse struct {
	// +demi:length chars min=1
	ID      string `json:"id"`
	Message string `json:"message"`
}

func (*ErrorResponse) machineResponse() {}

// WireMessage identifies a message of the machine-manager socket.
func (*ErrorResponse) WireMessage() {}

// A device's sandbox exited without being asked to stop.
// +demi:variant MachineResponse death
// +demi:tolerant
type Death struct {
	// +demi:length chars min=1
	DeviceID string `json:"deviceId"`
}

func (*Death) machineResponse() {}

// WireMessage identifies a message of the machine-manager socket.
func (*Death) WireMessage() {}
