package backendtest

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"testing"
	"time"

	"github.com/wspl/demi/internal/webapi"
)

// Base is the base every scripted device boots from.
const Base = "test-base"

// MachineScript is what the manager does differently, as a test sets it.
// Configure between calls; running requests retain the script they admitted.
type MachineScript struct {
	// Fail the next reset with this message.
	FailReset *string
	// Wakes start no runner, as a guest that never connects.
	SilentWake bool
	// Fail the next hibernate with this message.
	FailHibernate *string
	// CloudEnv replaces the environment of newly started Cloud runners;
	// nil inherits the test process's environment.
	CloudEnv map[string]string
	// The artifact cache each Cloud runner that starts from now on uses,
	// as `DEMI_ARTIFACTS`; none keeps its own in its state.
	Artifacts *string
}

// ScriptedManager owns a local machine-manager socket and its real runner
// processes. Test cleanup closes connections, stops runners and joins workers.
// It emulates machines without creating a VM or calling an external service.
type ScriptedManager struct{}

// StartScriptedManager starts a manager with temporary socket and device data.
func StartScriptedManager(ctx context.Context, t testing.TB) (*ScriptedManager, error) {
	panic("not written: b-backend")
}

// Socket returns the machine-manager Unix socket path.
func (m *ScriptedManager) Socket() string { panic("not written: b-backend") }

// Calls snapshots manager request names in arrival order.
func (m *ScriptedManager) Calls() []string { panic("not written: b-backend") }

// Arrival waits until call arrives and returns its recorded arrival time.
func (m *ScriptedManager) Arrival(ctx context.Context, call string) (time.Time, error) {
	panic("not written: b-backend")
}

// Count returns how many times call arrived.
func (m *ScriptedManager) Count(call string) int { panic("not written: b-backend") }

// SetScript replaces the future requests' script; maps are copied.
func (m *ScriptedManager) SetScript(script MachineScript) { panic("not written: b-backend") }

// Devices returns the manager's known device IDs.
func (m *ScriptedManager) Devices() []webapi.DeviceID { panic("not written: b-backend") }

// Running reports whether the scripted device has a running guest.
func (m *ScriptedManager) Running(device webapi.DeviceID) bool { panic("not written: b-backend") }

// State returns the device runner's installation-state directory.
func (m *ScriptedManager) State(device webapi.DeviceID) string { panic("not written: b-backend") }

// Home returns the device's preserved home directory.
func (m *ScriptedManager) Home(device webapi.DeviceID) string { panic("not written: b-backend") }

// Artifacts returns the device runner's artifact-cache directory.
func (m *ScriptedManager) Artifacts(device webapi.DeviceID) string { panic("not written: b-backend") }

// Kill stops a device runner and reports its death to the backend.
func (m *ScriptedManager) Kill(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-backend")
}

// StopQuietly stops a device runner without reporting a death event.
func (m *ScriptedManager) StopQuietly(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-backend")
}

// Close stops and joins manager connections, handlers and runner processes.
func (m *ScriptedManager) Close(ctx context.Context) error { panic("not written: b-backend") }

// HoldReset holds resets at their disk step until Release or test cleanup.
// UntilArrived waits for a reset to reach the held step.
func (m *ScriptedManager) HoldReset(t testing.TB) *StepHold { panic("not written: b-backend") }
