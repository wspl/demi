//go:build linux

package network

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/machines/system"
)

// namespacePath names the bind that keeps a Cloud slot's namespace alive.
func namespacePath(slot Slot) string { return filepath.Join("/run/netns", slot.Namespace()) }

// createNamespace binds a new Cloud namespace into the caller's mount namespace.
func createNamespace(ctx context.Context, path string) error {
	mounts, err := os.Open("/proc/thread-self/ns/mnt")
	if err != nil {
		return err
	}
	// Read-only namespace descriptors have no buffered writes to report.
	defer func() { _ = mounts.Close() }()
	_, err = system.RunNamespace(ctx, system.NewNetwork(), func(ctx context.Context) (struct{}, error) {
		network, err := os.Open("/proc/thread-self/ns/net")
		if err != nil {
			return struct{}{}, err
		}
		defer func() { _ = network.Close() }()
		return system.RunNamespace(ctx, system.Mount(mounts), func(ctx context.Context) (struct{}, error) {
			if err := os.MkdirAll(filepath.Dir(path), 0777); err != nil {
				return struct{}{}, err
			}
			file, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE|os.O_EXCL, 0444)
			if err != nil {
				return struct{}{}, err
			}
			if err := file.Close(); err != nil {
				return struct{}{}, err
			}
			err = system.Bind(ctx, fmt.Sprintf("/proc/self/fd/%d", network.Fd()), path)
			if err != nil {
				// The new namespace ends with its job. Preserve the bind failure; a
				// failed removal will be reported by the next creation, as in Rust.
				_ = os.Remove(path)
			}
			return struct{}{}, err
		})
	})
	return err
}

// removeNamespace releases the saved Cloud namespace if it still exists.
func removeNamespace(ctx context.Context, path string) error {
	mounted, _, err := system.MountRoot(ctx, path)
	if err != nil {
		return err
	}
	if mounted {
		if err := system.Detach(ctx, path); err != nil {
			return err
		}
	}
	if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return nil
}
