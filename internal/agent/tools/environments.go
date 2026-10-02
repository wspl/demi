package tools

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// Environments owns one node's shell environments and their repeat guards.
// Its zero value is ready for use. Concurrent calls for one Host create one
// environment. The owner must Dispose it and wait for cleanup before release.
type Environments struct{}

// PageViews returns the pages' view of every command held by the environments.
func (e *Environments) PageViews() []host.PageView { panic("not written: a-tools") }

// Owning returns the environment that owns command, or nil when none does.
func (e *Environments) Owning(command core.CommandID) host.ShellEnvironment {
	panic("not written: a-tools")
}

// EndAll ends every shell and forgets its environment, joining owned work.
// The next call on a Host makes a fresh environment. Use a cleanup context
// that remains usable after action cancellation.
func (e *Environments) EndAll(ctx context.Context) error { panic("not written: a-tools") }

// Dispose permanently closes the node's shells, including environments being
// created concurrently, and joins their work before returning. Use a cleanup
// context that remains usable after action cancellation.
func (e *Environments) Dispose(ctx context.Context) error { panic("not written: a-tools") }
