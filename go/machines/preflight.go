//go:build linux

package machines

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"uuid"

	"github.com/wspl/demi/go/machines/internal/fault"
	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/sandbox"
	"github.com/wspl/demi/go/machines/internal/storage"
)

// The checks before the manager serves (docs/cloud/setup.md § Linux
// requirements): privileges, a private mount namespace, the pinned runsc, one
// filesystem for the state directory, and a storage probe that mounts, freezes
// and copies a small image.

const (
	probePrefix = ".preflight-"
	probeBytes  = 32 << 20
)

// The refusals of a start.
var (
	// ErrPrivileges means the manager does not run as root.
	ErrPrivileges = errors.New("Cloud manager requires privileged Linux service execution")
	// ErrSharedNamespace means the manager shares the host's mount namespace.
	ErrSharedNamespace = errors.New("Cloud manager requires a private mount namespace (systemd PrivateMounts=yes)")
)

// A RunscVersionError means runsc does not report the pinned version.
type RunscVersionError struct {
	Version string
}

func (e *RunscVersionError) Error() string { return "Cloud requires runsc " + e.Version }

// A FilesystemsError means the working and image directories are on different
// filesystems.
type FilesystemsError struct {
	Working string
	Images  string
}

func (e *FilesystemsError) Error() string {
	return fmt.Sprintf("DEMI_MACHINES_DATA must be one filesystem: %s and %s are on different filesystems", e.Working, e.Images)
}

// requireRoot requires root, which the mounts, namespaces and firewall need.
func requireRoot() error {
	if os.Geteuid() != 0 {
		return ErrPrivileges
	}
	return nil
}

// requirePrivateNamespace requires a mount namespace of the manager's own, which
// keeps its storage mounts off the host.
func requirePrivateNamespace() error {
	own, err := os.Stat("/proc/self/ns/mnt")
	if err != nil {
		return err
	}
	host, err := os.Stat("/proc/1/ns/mnt")
	if err != nil {
		return err
	}
	if os.SameFile(own, host) {
		return ErrSharedNamespace
	}
	return nil
}

// requireRunsc requires the configured runsc to report exactly the pinned
// version.
func requireRunsc(ctx context.Context, core *Core) error {
	version := sandbox.PinnedRelease().Version()
	reported, err := core.Runsc.Version(ctx)
	if err != nil {
		return err
	}
	if !sandbox.ReportsVersion(reported, version) {
		return &RunscVersionError{Version: version}
	}
	return nil
}

// requireOneFilesystem requires the working and image directories on one
// filesystem, because publication links images between them.
func requireOneFilesystem(working, images string) error {
	if err := os.MkdirAll(working, 0o777); err != nil {
		return err
	}
	if err := os.MkdirAll(images, 0o777); err != nil {
		return err
	}
	workingDevice, err := device(working)
	if err != nil {
		return err
	}
	imagesDevice, err := device(images)
	if err != nil {
		return err
	}
	if workingDevice != imagesDevice {
		return &FilesystemsError{Working: working, Images: images}
	}
	return nil
}

// device returns the device number of the filesystem holding path.
func device(path string) (uint64, error) {
	info, err := os.Stat(path)
	if err != nil {
		return 0, err
	}
	return info.Sys().(*syscall.Stat_t).Dev, nil
}

// probeStorage makes a small image, mounts it through a loop device under an
// overlay, writes through the overlay, freezes the filesystem and copies the
// image; the probe is released whatever happens.
func probeStorage(ctx context.Context, core *Core) error {
	stage := filepath.Join(core.Config.Data, probePrefix+uuid.New().String())
	probed := probe(ctx, core, stage)
	released := releaseProbe(stage)
	return errors.Join(probed, released)
}

func probe(ctx context.Context, core *Core, stage string) error {
	if err := os.Mkdir(stage, 0o777); err != nil {
		return err
	}
	for _, name := range []string{"volume", "merged", "base"} {
		if err := os.Mkdir(filepath.Join(stage, name), 0o777); err != nil {
			return err
		}
	}
	image := filepath.Join(stage, "volume.ext4")
	if err := storage.MakeSystem(ctx, core.Tools, image, probeBytes); err != nil {
		return err
	}
	volume := filepath.Join(stage, "volume")
	device, err := linux.AttachLoop(image)
	if err != nil {
		return err
	}
	mounted := linux.MountExt4(device.Path(), volume)
	device.Close()
	if mounted != nil {
		return mounted
	}
	if err := linux.MountSystemOverlay(filepath.Join(stage, "base"), volume, filepath.Join(stage, "merged")); err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(stage, "merged/probe"), []byte("Cloud storage preflight"), 0o666); err != nil {
		return err
	}
	if err := linux.Freeze(volume); err != nil {
		return err
	}
	fault.Point("probe-frozen")
	return storage.CloneSparse(image, filepath.Join(stage, "copy.ext4"))
}

// releaseProbe thaws and unmounts a probe, then removes it.
func releaseProbe(stage string) error {
	volume := filepath.Join(stage, "volume")
	root, err := linux.IsMountRoot(volume)
	if err != nil {
		return err
	}
	if root {
		if _, err := linux.Thaw(volume); err != nil {
			return err
		}
	}
	for _, name := range []string{"merged", "volume"} {
		path := filepath.Join(stage, name)
		root, err := linux.IsMountRoot(path)
		if err != nil {
			return err
		}
		if root {
			if err := linux.Unmount(path); err != nil {
				return err
			}
		}
	}
	return storage.RemoveTree(stage)
}

// releaseProbes releases the probes an interrupted manager left in data.
func releaseProbes(data string) error {
	entries, err := os.ReadDir(data)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), probePrefix) {
			if err := releaseProbe(filepath.Join(data, entry.Name())); err != nil {
				return err
			}
		}
	}
	return nil
}
