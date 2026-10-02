//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
)

// ImportBase verifies and imports the release in image into bases unless it
// is already there, and returns its manifest digest as the base version.
// It never executes image content on the host.
func ImportBase(ctx context.Context, tools *system.Tools, image, bases string) (machinewire.BaseVersion, error) {
	panic("not written: m-storage")
}
