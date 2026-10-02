//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/machines/system"
)

// Capacity reads block count times block size from the primary ext4 superblock.
func Capacity(ctx context.Context, image string) (uint64, error) { panic("not written: m-storage") }

// MakeSystem creates an empty system filesystem. bytes must be nonzero.
// Overlay upper and work directories are made when it is first mounted.
func MakeSystem(ctx context.Context, tools *system.Tools, image string, bytes uint64) error {
	panic("not written: m-storage")
}

// MakeHome creates a home filesystem populated from root, including ownership
// and modes. bytes must be nonzero.
func MakeHome(ctx context.Context, tools *system.Tools, root, image string, bytes uint64) error {
	panic("not written: m-storage")
}

// GrowMounted grows the mounted ext4 filesystem to device's size, without a
// deadline. A failure caused by missing CAP_SYS_RESOURCE names the capability.
func GrowMounted(ctx context.Context, tools *system.Tools, device string) error {
	panic("not written: m-storage")
}

// Recover checks an unmounted image, completes interrupted growth to its file
// size, syncs it, and returns its actual capacity. Checks and resizes have no
// deadline and are allowed to finish once started, including on cancellation.
func Recover(ctx context.Context, tools *system.Tools, image string) (uint64, error) {
	panic("not written: m-storage")
}
