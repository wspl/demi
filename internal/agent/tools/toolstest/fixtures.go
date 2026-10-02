package toolstest

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/host"
)

// NoHost is the Host type for test products whose agents run no shell tools.
// It also implements tools.HostResolver[*NoHost], refusing Host resolution.
// Its Host methods must never be called: no environment is made on it.
type NoHost struct{}

// Host refuses shell access because this agent runs no shell tools.
func (h *NoHost) Host(ctx context.Context, node tools.NodeContext) (*NoHost, error) {
	panic("not written: a-tools")
}

// Key is unreachable for a product with no shell Host.
func (h *NoHost) Key() host.Key { panic("not written: a-tools") }

// DefaultCWD is unreachable for a product with no shell Host.
func (h *NoHost) DefaultCWD() string { panic("not written: a-tools") }

// Identity is unreachable for a product with no shell Host.
func (h *NoHost) Identity() host.Identity { panic("not written: a-tools") }

// FS is unreachable for a product with no shell Host.
func (h *NoHost) FS() host.FS { panic("not written: a-tools") }

// Process is unreachable for a product with no shell Host.
func (h *NoHost) Process() host.Process { panic("not written: a-tools") }

// NoShells is the shell environment factory of a product whose Host is NoHost.
type NoShells struct{}

// Create cannot be reached by an agent that runs no shell tools.
func (s NoShells) Create(ctx context.Context, scope tools.EnvironmentScope, target *NoHost) (host.ShellEnvironment, error) {
	panic("not written: a-tools")
}

// Field returns a shell tool result's name: value line, such as commandId.
// It panics when the result has no such field.
func Field(result, name string) string { panic("not written: a-tools") }

// ShownOutput returns displayed output lines, each with a newline, or empty
// text when none are shown. Repeated unfinished lines remain repeated; running
// status, the newest-output note and the next step are excluded.
func ShownOutput(result string) string { panic("not written: a-tools") }

var (
	_ host.Host                              = (*NoHost)(nil)
	_ tools.HostResolver[*NoHost]            = (*NoHost)(nil)
	_ tools.ShellEnvironmentFactory[*NoHost] = NoShells{}
)
