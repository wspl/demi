//go:build linux

package systemtest

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"golang.org/x/sys/unix"
)

// LoopAttached reports whether the kernel has a loop device backed by image.
// The image must still exist so its canonical path can be compared with sysfs.
func LoopAttached(ctx context.Context, image string) (bool, error) {
	if err := ctx.Err(); err != nil {
		return false, err
	}
	canonical, err := filepath.EvalSymlinks(image)
	if err != nil {
		return false, err
	}
	canonical, err = filepath.Abs(canonical)
	if err != nil {
		return false, err
	}
	entries, err := os.ReadDir("/sys/block")
	if err != nil {
		return false, err
	}
	for _, entry := range entries {
		if err := ctx.Err(); err != nil {
			return false, err
		}
		data, err := os.ReadFile(filepath.Join("/sys/block", entry.Name(), "loop/backing_file"))
		// Auto-clear may remove the association between readdir and read.
		if errors.Is(err, os.ErrNotExist) || errors.Is(err, unix.ENODEV) {
			continue
		}
		if err != nil {
			return false, err
		}
		if strings.TrimSpace(string(data)) == canonical {
			return true, nil
		}
	}
	return false, nil
}

// WaitLoopDetach waits until image has no attached loop device. It polls the
// kernel condition without sleeps, with a five-second hang deadline and the
// caller's cancellation. The image must remain present until this returns.
func WaitLoopDetach(ctx context.Context, image string) error {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	for {
		attached, err := LoopAttached(ctx, image)
		if err != nil {
			return fmt.Errorf("waiting for loop detach of %s: %w", image, err)
		}
		if !attached {
			return nil
		}
		runtime.Gosched()
	}
}
