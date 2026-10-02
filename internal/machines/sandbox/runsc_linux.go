//go:build linux

package sandbox

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"os"

	"github.com/wspl/demi/internal/machines/system"
)

// ProfileFlags returns the fixed systrap, sandbox network, shared filesystem,
// setuid and Directfs profile. Each call returns an independent array.
func ProfileFlags() [7]string { panic("not written: m-sandbox") }

// PinnedRelease returns the runtime release built into the manager.
func PinnedRelease() RuntimeRelease { panic("not written: m-sandbox") }

// Version returns the pinned version for this architecture, including the
// seccomp trap fix on arm64.
func (r RuntimeRelease) Version() string { panic("not written: m-sandbox") }

// ReportsVersion checks whether the first output line reports exactly version.
func ReportsVersion(output, version string) bool { panic("not written: m-sandbox") }

// StatusIn reads runsc list output. found is false for an absent container,
// including a null list; unknown fields in container entries are ignored.
func StatusIn(listing []byte, id ID) (status Status, found bool, err error) {
	panic("not written: m-sandbox")
}

// Runsc drives the pinned runtime with state under the manager's runtime directory.
type Runsc struct{}

// NewRunsc selects the resolved tools, runtime root and explicit cgroup mode.
func NewRunsc(tools *system.Tools, runtime string, cgroups bool) *Runsc {
	panic("not written: m-sandbox")
}

// Root returns where runsc keeps its containers' state.
func (r *Runsc) Root() string { panic("not written: m-sandbox") }

// Args prepends the state root and fixed profile, including --ignore-cgroups
// when resource limits are off.
func (r *Runsc) Args(command []string) []string { panic("not written: m-sandbox") }

// Version returns the installed runsc's version output.
func (r *Runsc) Version(ctx context.Context) (string, error) { panic("not written: m-sandbox") }

// Status reports the container's state; found is false when runsc does not know it.
func (r *Runsc) Status(ctx context.Context, id ID) (status Status, found bool, err error) {
	panic("not written: m-sandbox")
}

// Start starts a detached container from bundle, writing stdout and stderr to
// log. The caller keeps ownership of log and closes it after Start returns.
// Failure reports the last 8 KiB from logPath. Cancellation kills and reaps
// the runsc command; the boot owner must still close the partial sandbox.
func (r *Runsc) Start(ctx context.Context, id ID, bundle string, log *os.File, logPath string) error {
	panic("not written: m-sandbox")
}

// Wait waits for a container to exit without a deadline. Cancellation kills
// and reaps the wait command, leaving the sandbox for its owner to close.
func (r *Runsc) Wait(ctx context.Context, id ID) error { panic("not written: m-sandbox") }

// Pause suspends sandbox execution.
func (r *Runsc) Pause(ctx context.Context, id ID) error { panic("not written: m-sandbox") }

// Resume resumes suspended sandbox execution.
func (r *Runsc) Resume(ctx context.Context, id ID) error { panic("not written: m-sandbox") }

// Terminate asks every sandbox process to terminate and returns even a nonzero status.
func (r *Runsc) Terminate(ctx context.Context, id ID) (system.Output, error) {
	panic("not written: m-sandbox")
}

// Delete deletes the container and kills whatever still runs.
func (r *Runsc) Delete(ctx context.Context, id ID) error { panic("not written: m-sandbox") }
