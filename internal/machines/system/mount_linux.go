//go:build linux

package system

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "context"

// MountRoot reports whether path is a mount root and whether it exists, without following symlinks or automounting.
func MountRoot(ctx context.Context, path string) (mounted, exists bool, err error) {
	panic("not written: m-system")
}

// Bind binds source onto target.
func Bind(ctx context.Context, source, target string) error { panic("not written: m-system") }

// RemountReadOnly makes the bind at target read-only.
func RemountReadOnly(ctx context.Context, target string) error { panic("not written: m-system") }

// Ext4 mounts the ext4 filesystem on device at target, without device files.
func Ext4(ctx context.Context, device, target string) error { panic("not written: m-system") }

// Tmpfs mounts a tmpfs at target with options, such as size=1m,mode=0700.
func Tmpfs(ctx context.Context, target, options string) error { panic("not written: m-system") }

// Overlay mounts base below volume/upper and volume/work at target. A new upper directory
// gets mode 0755; an existing one keeps the mode its user set.
func Overlay(ctx context.Context, base, volume, target string) error {
	panic("not written: m-system")
}

// Unmount unmounts target.
func Unmount(ctx context.Context, target string) error { panic("not written: m-system") }

// Detach detaches target now; the kernel releases it when its last user goes.
func Detach(ctx context.Context, target string) error { panic("not written: m-system") }

// MakePrivate stops mount events propagating to and from target.
func MakePrivate(ctx context.Context, target string) error { panic("not written: m-system") }
