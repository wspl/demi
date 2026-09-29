//go:build linux

package linux

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"

	"golang.org/x/sys/unix"
)

// IsMountRoot reports whether path is the root of a mount; a path where nothing
// is reports false.
func IsMountRoot(path string) (bool, error) {
	var status unix.Statx_t
	err := unix.Statx(unix.AT_FDCWD, path, unix.AT_NO_AUTOMOUNT|unix.AT_SYMLINK_NOFOLLOW, 0, &status)
	switch {
	case err == nil:
	case errors.Is(err, unix.ENOENT):
		return false, nil
	default:
		return false, Failed("inspecting", path, err)
	}
	if status.Mask&unix.STATX_BASIC_STATS == 0 || status.Attributes_mask&unix.STATX_ATTR_MOUNT_ROOT == 0 {
		return false, errors.New("the kernel does not report mount roots (Linux 5.8 or later is required)")
	}
	return status.Attributes&unix.STATX_ATTR_MOUNT_ROOT != 0, nil
}

// Bind binds source onto target.
func Bind(source, target string) error {
	if err := unix.Mount(source, target, "", unix.MS_BIND, ""); err != nil {
		return Failed("binding", target, err)
	}
	return nil
}

// RemountReadOnly makes the bind at target read-only.
func RemountReadOnly(target string) error {
	if err := unix.Mount("", target, "", unix.MS_REMOUNT|unix.MS_BIND|unix.MS_RDONLY, ""); err != nil {
		return Failed("remounting", target, err)
	}
	return nil
}

// MountExt4 mounts the ext4 filesystem on device at target, without device
// files.
func MountExt4(device, target string) error {
	if err := unix.Mount(device, target, "ext4", unix.MS_NODEV, ""); err != nil {
		return Failed("mounting", target, err)
	}
	return nil
}

// MountTmpfs mounts a tmpfs at target with options, such as "size=1m,mode=0700".
func MountTmpfs(target, options string) error {
	if err := unix.Mount("tmpfs", target, "tmpfs", 0, options); err != nil {
		return Failed("mounting", target, err)
	}
	return nil
}

// MountSystemOverlay mounts the system overlay: the read-only base below the
// upper and work directories of the system filesystem mounted at volume. A new
// upper directory gets mode 0755, because OverlayFS presents it as the sandbox's
// / and the service's umask would hide it; an existing one keeps the mode its
// user set.
func MountSystemOverlay(base, volume, target string) error {
	upper := filepath.Join(volume, "upper")
	work := filepath.Join(volume, "work")
	if err := createWithMode(upper, 0o755); err != nil {
		return err
	}
	if err := createWithMode(work, 0o700); err != nil {
		return err
	}
	options := fmt.Sprintf("lowerdir=%s,upperdir=%s,workdir=%s,index=off,metacopy=off,redirect_dir=off", base, upper, work)
	if err := unix.Mount("overlay", target, "overlay", 0, options); err != nil {
		return Failed("mounting", target, err)
	}
	return nil
}

// createWithMode creates the directory path with exactly mode, or leaves an
// existing one as it is.
func createWithMode(path string, mode fs.FileMode) error {
	err := os.Mkdir(path, mode)
	switch {
	case err == nil:
		return os.Chmod(path, mode)
	case errors.Is(err, fs.ErrExist):
		return nil
	}
	return err
}

// Unmount unmounts target.
func Unmount(target string) error {
	if err := unix.Unmount(target, 0); err != nil {
		return Failed("unmounting", target, err)
	}
	return nil
}

// Detach detaches the mount at target now; the kernel releases it when its last
// user goes, as ip netns delete does with a namespace file.
func Detach(target string) error {
	if err := unix.Unmount(target, unix.MNT_DETACH); err != nil {
		return Failed("unmounting", target, err)
	}
	return nil
}

// MakePrivate stops mount events propagating to and from target.
func MakePrivate(target string) error {
	if err := unix.Mount("", target, "", unix.MS_PRIVATE, ""); err != nil {
		return Failed("changing the propagation of", target, err)
	}
	return nil
}
