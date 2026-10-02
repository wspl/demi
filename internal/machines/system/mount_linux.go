//go:build linux

package system

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/unix"
)

// MountRoot reports whether path is a mount root and whether it exists, without following symlinks or automounting.
func MountRoot(ctx context.Context, path string) (mounted, exists bool, err error) {
	if err := ctx.Err(); err != nil {
		return false, false, err
	}
	var status unix.Statx_t
	err = unix.Statx(unix.AT_FDCWD, path, unix.AT_NO_AUTOMOUNT|unix.AT_SYMLINK_NOFOLLOW, 0, &status)
	if errors.Is(err, unix.ENOENT) {
		return false, false, nil
	}
	if err != nil {
		return false, false, Failed("inspecting", path, err)
	}
	if status.Attributes_mask&unix.STATX_ATTR_MOUNT_ROOT == 0 {
		return false, true, mountRootUnsupported{}
	}
	return status.Attributes&unix.STATX_ATTR_MOUNT_ROOT != 0, true, nil
}

// Bind binds source onto target.
func Bind(ctx context.Context, source, target string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return Failed("binding", target, unix.Mount(source, target, "", unix.MS_BIND, ""))
}

// RemountReadOnly makes the bind at target read-only.
func RemountReadOnly(ctx context.Context, target string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return Failed("remounting", target, unix.Mount("", target, "", unix.MS_BIND|unix.MS_RDONLY|unix.MS_REMOUNT, ""))
}

// Ext4 mounts the ext4 filesystem on device at target, without device files.
func Ext4(ctx context.Context, device, target string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return Failed("mounting", target, unix.Mount(device, target, "ext4", unix.MS_NODEV, ""))
}

// Tmpfs mounts a tmpfs at target with options, such as size=1m,mode=0700.
func Tmpfs(ctx context.Context, target, options string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return Failed("mounting", target, unix.Mount("tmpfs", target, "tmpfs", 0, options))
}

// Overlay mounts base below volume/upper and volume/work at target. A new upper directory
// gets mode 0755; an existing one keeps the mode its user set.
func Overlay(ctx context.Context, base, volume, target string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	upper, work := filepath.Join(volume, "upper"), filepath.Join(volume, "work")
	for _, directory := range []struct {
		path string
		mode os.FileMode
	}{{upper, 0755}, {work, 0700}} {
		err := os.Mkdir(directory.path, 0777)
		if errors.Is(err, os.ErrExist) {
			continue
		}
		if err != nil {
			return err
		}
		if err := os.Chmod(directory.path, directory.mode); err != nil {
			return err
		}
	}
	options := fmt.Sprintf("lowerdir=%s,upperdir=%s,workdir=%s,index=off,metacopy=off,redirect_dir=off", base, upper, work)
	return Failed("mounting", target, unix.Mount("overlay", target, "overlay", 0, options))
}

// Unmount unmounts target.
func Unmount(ctx context.Context, target string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return Failed("unmounting", target, unix.Unmount(target, 0))
}

// Detach detaches target now; the kernel releases it when its last user goes.
func Detach(ctx context.Context, target string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return Failed("unmounting", target, unix.Unmount(target, unix.MNT_DETACH))
}

// MakePrivate stops mount events propagating to and from target.
func MakePrivate(ctx context.Context, target string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return Failed("changing the propagation of", target, unix.Mount("", target, "", unix.MS_PRIVATE, ""))
}

// mountRootUnsupported keeps the Rust diagnostic while exposing its error kind.
type mountRootUnsupported struct{}

func (mountRootUnsupported) Error() string {
	return "the kernel does not report mount roots (Linux 5.8 or later is required)"
}

func (mountRootUnsupported) Unwrap() error { return errors.ErrUnsupported }
