//go:build linux

package storage

import (
	"context"

	"golang.org/x/sys/unix"

	"github.com/wspl/demi/go/machines/internal/tools"
)

// GrowMounted grows the mounted ext4 filesystem on device to the device's size.
// resize2fs reports the kernel's refusal for want of CAP_SYS_RESOURCE as a bare
// "Permission denied"; the error names the capability instead
// (docs/cloud/managed-hosts.md § Lifecycle and capacity).
func GrowMounted(ctx context.Context, t *tools.Tools, device string) error {
	_, failed := t.Run(ctx, tools.Resize2fs, []string{device}, 0)
	if failed == nil {
		return nil
	}
	// The manager runs as root, so resize2fs holds what the manager's bounding
	// set allows.
	held, err := unix.PrctlRetInt(unix.PR_CAPBSET_READ, unix.CAP_SYS_RESOURCE, 0, 0, 0)
	if err != nil {
		return err
	}
	if held == 0 {
		return &GrowthError{Resize: failed}
	}
	return failed
}
