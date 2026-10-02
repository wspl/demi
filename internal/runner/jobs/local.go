//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobs

import (
	"context"
	"net"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// Server owns the private local endpoint and every command client it accepts.
// Its owner must Close it, including after the lifetime context is cancelled.
type Server struct{}

// StartServer binds the endpoint and serves handler until ctx ends or Close runs.
func StartServer(ctx context.Context, handler cmdsdk.Handler[commandwire.LocalInvocation]) (*Server, error) {
	panic("not written: r-jobs")
}

// Endpoint returns the command clients' endpoint.
func (s *Server) Endpoint() string { panic("not written: r-jobs") }

// WaitIdle waits until no local clients remain connected.
func (s *Server) WaitIdle(ctx context.Context) error { panic("not written: r-jobs") }

// Close stops accepting, cancels and joins clients, and removes the endpoint.
// If ctx ends first, shutdown continues and a later Close can join it.
func (s *Server) Close(ctx context.Context) error { panic("not written: r-jobs") }

// Listener owns a private socket and its liveness lock on Unix, or the local
// endpoint on Windows. Its owner must Close it; accepted connections belong to callers.
type Listener struct{}

// BindListener creates a local endpoint and its platform-specific liveness resources.
func BindListener(ctx context.Context) (*Listener, error) { panic("not written: r-jobs") }

// Endpoint returns the local address for forwarding clients.
func (l *Listener) Endpoint() string { panic("not written: r-jobs") }

// Accept waits for a command connection. Cancelling ctx interrupts the wait.
func (l *Listener) Accept(ctx context.Context) (net.Conn, error) { panic("not written: r-jobs") }

// Close releases the endpoint and liveness resources and wakes Accept.
func (l *Listener) Close() error { panic("not written: r-jobs") }
