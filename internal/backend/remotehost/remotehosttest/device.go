package remotehosttest

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

// TestDeviceID names the device a test connection serves.
const TestDeviceID = "test-device"

// CommandPolicy runs all calls in one command set with each node's storage in memory.
type CommandPolicy struct{ _ byte }

// NewCommandPolicy constructs the policy for commands.
func NewCommandPolicy(commands *host.CommandSet) *CommandPolicy { panic("not written: b-remotehost") }

// NodeStorage returns the command storage of node.
func (p *CommandPolicy) NodeStorage(node string) *hosttest.MemoryStorage {
	panic("not written: b-remotehost")
}

// AdmitCall admits every test call.
func (p *CommandPolicy) AdmitCall(job remotehost.JobOrigin) error { panic("not written: b-remotehost") }

// Dispatch runs invocation in the test command set.
func (p *CommandPolicy) Dispatch(ctx context.Context, job remotehost.JobOrigin, invocation host.RPCInvocation, port host.RPCPort) (uint8, error) {
	panic("not written: b-remotehost")
}

// Storage operates on the caller node's storage while the call lives.
func (p *CommandPolicy) Storage(ctx context.Context, job remotehost.JobOrigin, op host.StorageOp) (host.StorageReply, error) {
	panic("not written: b-remotehost")
}

// GrowVolume refuses growth, which is unavailable in this fixture.
func (p *CommandPolicy) GrowVolume(ctx context.Context, volume runnerwire.VolumeName, bytes uint64) error {
	panic("not written: b-remotehost")
}

// ReserveNumbers counts each conversation's sequence from one.
func (p *CommandPolicy) ReserveNumbers(ctx context.Context, conversation string, sequence commandwire.ServiceSequence, count uint32) (uint64, error) {
	panic("not written: b-remotehost")
}

// TestDevice is an in-process fake runner whose connections are TestLinks.
// Construction registers cleanup for all links and the pipe broker.
type TestDevice struct{ _ byte }

// NewTestDevice creates a device with policy and registers test cleanup.
func NewTestDevice(t testing.TB, policy remotehost.LinkPolicy) *TestDevice {
	panic("not written: b-remotehost")
}

// Pipes returns the device's pipe broker.
func (d *TestDevice) Pipes() *remotehost.Pipes { panic("not written: b-remotehost") }

// Host constructs a Host on this device with cwd and admission.
func (d *TestDevice) Host(cwd string, admission remotehost.Admission) *remotehost.Host {
	panic("not written: b-remotehost")
}

// Connect connects a fake runner. Nil ping disables liveness.
func (d *TestDevice) Connect(ping *time.Duration) *TestLink { panic("not written: b-remotehost") }

// Close closes all connections and pipes and joins their workers.
func (d *TestDevice) Close(ctx context.Context) error { panic("not written: b-remotehost") }

// TestLink drives the real Link engine with an in-process runner's frames.
type TestLink struct{ _ byte }

// Link returns the backend connection handle.
func (l *TestLink) Link() *remotehost.Link { panic("not written: b-remotehost") }

// Next waits for and decodes the backend's next message.
func (l *TestLink) Next(ctx context.Context) (runnerwire.Inbound, error) {
	panic("not written: b-remotehost")
}

// TryNext takes a queued message without waiting; false means no queued frame.
func (l *TestLink) TryNext() (runnerwire.Inbound, bool, error) { panic("not written: b-remotehost") }

// Send encodes and sends a runner message to the backend.
func (l *TestLink) Send(ctx context.Context, message runnerwire.Outbound) error {
	panic("not written: b-remotehost")
}

// SendFrame sends raw bytes, including malformed frames for refusal tests.
func (l *TestLink) SendFrame(ctx context.Context, frame []byte) error {
	panic("not written: b-remotehost")
}

// Close disconnects the runner and joins its driver.
func (l *TestLink) Close(ctx context.Context) (remotehost.LinkEnd, error) {
	panic("not written: b-remotehost")
}

// Ended waits for the driver to finish and joins it.
func (l *TestLink) Ended(ctx context.Context) (remotehost.LinkEnd, error) {
	panic("not written: b-remotehost")
}

var _ remotehost.LinkPolicy = (*CommandPolicy)(nil)
