//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobs

import (
	"context"

	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// ServiceStreams owns user calls on resident services, their pipe IO and the
// connection's last package bindings. Its owner must Close it before services stop.
type ServiceStreams struct{}

// NewServiceStreams creates an owner that cancels streams with ctx. Closing
// draining refuses new calls while retaining the service binding, as Rust does.
func NewServiceStreams(ctx context.Context, connection *ConnectionHandle, pipes *process.PipeClient, services *cmdpkgs.ServiceHandle, draining <-chan struct{}) *ServiceStreams {
	panic("not written: r-jobs")
}

// HandleOpen registers a service_open request and starts its owned work without
// waiting for completion. It refuses other messages and a closed connection.
func (s *ServiceStreams) HandleOpen(message runnerwire.Inbound) error { panic("not written: r-jobs") }

// Close cancels and joins every stream, then releases its service bindings.
// If ctx ends first, a later Close can join shutdown. It is idempotent.
func (s *ServiceStreams) Close(ctx context.Context) error { panic("not written: r-jobs") }
