//go:build linux

package machinemanager

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/wspl/demi/internal/machinemanager/sandbox"
	"github.com/wspl/demi/internal/machinemanager/storage"
	"github.com/wspl/demi/internal/machinemanager/system"
	"golang.org/x/sys/unix"
)

// RequireRoot checks the privileges mounts, namespaces and the firewall need.
func RequireRoot() error {
	if os.Geteuid() != 0 {
		//nolint:staticcheck // User-visible text, kept byte for byte.
		return errors.New("Cloud manager requires privileged Linux service execution")
	}
	return nil
}

// RequirePrivateNamespace keeps storage mounts off the host mount namespace.
func RequirePrivateNamespace() error {
	own, err := os.Stat("/proc/self/ns/mnt")
	if err != nil {
		return err
	}
	host, err := os.Stat("/proc/1/ns/mnt")
	if err != nil {
		return err
	}
	if os.SameFile(own, host) {
		//nolint:staticcheck // User-visible text, kept byte for byte.
		return errors.New("Cloud manager requires a private mount namespace (systemd PrivateMounts=yes)")
	}
	return nil
}

// RequireRunsc verifies the exact pinned runtime version.
func RequireRunsc(ctx context.Context, core *Core) error {
	version := sandbox.PinnedRelease().Version()
	reported, err := core.Runsc.Version(ctx)
	if err != nil {
		return err
	}
	if !sandbox.ReportsVersion(reported, version) {
		//nolint:staticcheck // User-visible text, kept byte for byte.
		return fmt.Errorf("Cloud requires runsc %s", version)
	}
	return nil
}

// RequireOneFilesystem verifies publication can link images between directories.
func RequireOneFilesystem(ctx context.Context, working, images string) error {
	var devices [2]uint64
	for i, path := range []string{working, images} {
		if err := ctx.Err(); err != nil {
			return err
		}
		if err := os.MkdirAll(path, 0o777); err != nil {
			return err
		}
		var stat unix.Stat_t
		if err := unix.Stat(path, &stat); err != nil {
			return err
		}
		devices[i] = uint64(stat.Dev)
	}
	if devices[0] != devices[1] {
		//nolint:staticcheck // User-visible text, kept byte for byte.
		return fmt.Errorf(
			"DEMI_MACHINE_MANAGER_DATA must be one filesystem: %s and %s are on different filesystems",
			working,
			images,
		)
	}
	return nil
}

const probePrefix = ".preflight-"

// ProbeStorage exercises overlay writes and frozen sparse copies, always releasing its resources.
func ProbeStorage(ctx context.Context, core *Core) (err error) {
	id, err := storage.NewGeneration()
	if err != nil {
		return err
	}
	stage := filepath.Join(core.Config.Data, probePrefix+string(id))
	if err = os.Mkdir(stage, 0o700); err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, releaseProbe(context.WithoutCancel(ctx), stage))
	}()
	for _, name := range []string{"volume", "merged", "base"} {
		if err = os.Mkdir(filepath.Join(stage, name), 0o700); err != nil {
			return err
		}
	}
	image := filepath.Join(stage, "volume.ext4")
	if err = storage.MakeSystem(ctx, core.Tools, image, 32<<20); err != nil {
		return err
	}
	device, err := system.Attach(ctx, image)
	if err != nil {
		return err
	}
	// The loop descriptor is released even when mounting fails; autoclear owns detachment.
	defer func() {
		err = errors.Join(err, device.Close())
	}()
	volume := filepath.Join(stage, "volume")
	if err = system.Ext4(ctx, device.Path(), volume); err != nil {
		return err
	}
	if err = system.Overlay(ctx, filepath.Join(stage, "base"), volume, filepath.Join(stage, "merged")); err != nil {
		return err
	}
	if err = os.WriteFile(filepath.Join(stage, "merged/probe"), []byte("Cloud storage preflight"), 0o600); err != nil {
		return err
	}
	if err = system.Freeze(ctx, volume); err != nil {
		return err
	}
	system.FaultPoint("probe-frozen")
	return storage.CloneSparse(context.WithoutCancel(ctx), image, filepath.Join(stage, "copy.ext4"))
}

// releaseProbe thaws and unmounts a manager's storage probe before removing it.
func releaseProbe(ctx context.Context, stage string) error {
	volume := filepath.Join(stage, "volume")
	mounted, _, err := system.MountRoot(ctx, volume)
	if err != nil {
		return err
	}
	if mounted {
		if _, err = system.Thaw(ctx, volume); err != nil {
			return err
		}
	}
	for _, name := range []string{"merged", "volume"} {
		path := filepath.Join(stage, name)
		mounted, _, err = system.MountRoot(ctx, path)
		if err != nil {
			return err
		}
		if mounted {
			if err = system.Unmount(ctx, path); err != nil {
				return err
			}
		}
	}
	return storage.RemoveTree(ctx, stage)
}

// ReleaseProbes releases probes an interrupted manager left in data.
func ReleaseProbes(ctx context.Context, data string) error {
	entries, err := os.ReadDir(data)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), probePrefix) {
			if err = releaseProbe(ctx, filepath.Join(data, entry.Name())); err != nil {
				return err
			}
		}
	}
	return nil
}
