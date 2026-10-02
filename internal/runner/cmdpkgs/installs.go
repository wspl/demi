//revive:disable:unused-parameter API checkpoint retains parameter names for callers; bodies follow after merge.

package cmdpkgs

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
)

// Installs tracks downloads and unpacking in progress. Its zero value is usable.
// Share a pointer rather than copying it after first use.
type Installs struct{}

// Subscribe observes the current installs and subsequent changes.
func (i *Installs) Subscribe() *InstallsReceiver { panic("not written: r-cmdpkgs") }

// Close ends reporting and wakes receivers after the owner has joined its installs.
// Repeated calls do nothing.
func (i *Installs) Close() { panic("not written: r-cmdpkgs") }

// InstallsReceiver observes installation snapshots. Each observer has its own cursor.
type InstallsReceiver struct{}

// Current reads and marks the current list seen, keeping the oldest installs
// when there are more than a wire message can carry.
func (r *InstallsReceiver) Current() []runnerwire.Install { panic("not written: r-cmdpkgs") }

// Changed waits past the last observed list. It returns false when reporting ends,
// or an error when ctx is cancelled.
func (r *InstallsReceiver) Changed(ctx context.Context) (bool, error) {
	panic("not written: r-cmdpkgs")
}
