//go:build linux

package storage

import (
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/machines/system"
)

// CopySkeleton copies skeleton to a new destination, preserving symbolic links
// and modes and assigning every entry to the sandbox user.
func CopySkeleton(ctx context.Context, skeleton, destination string) error {
	return filepath.WalkDir(skeleton, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if err := ctx.Err(); err != nil {
			return err
		}
		relative, err := filepath.Rel(skeleton, path)
		if err != nil {
			return err
		}
		target := filepath.Join(destination, relative)
		switch {
		case entry.IsDir():
			info, err := entry.Info()
			if err != nil {
				return err
			}
			if err := os.Mkdir(target, 0777); err != nil {
				return err
			}
			if err := os.Chmod(target, info.Mode()); err != nil {
				return err
			}
		case entry.Type()&os.ModeSymlink != 0:
			link, err := os.Readlink(path)
			if err != nil {
				return err
			}
			if err := os.Symlink(link, target); err != nil {
				return err
			}
		case entry.Type().IsRegular():
			if err := copySkeletonFile(ctx, path, target); err != nil {
				return err
			}
		default:
			return fmt.Errorf("%s is neither a file, a directory nor a link: %w", path, os.ErrInvalid)
		}
		return os.Lchown(target, int(system.UserID), int(system.UserID))
	})
}

// copySkeletonFile copies a new home's regular file with its original permissions.
func copySkeletonFile(ctx context.Context, source, destination string) (err error) {
	if err := ctx.Err(); err != nil {
		return err
	}
	from, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() { _ = from.Close() }() // Read-only skeleton file.
	info, err := from.Stat()
	if err != nil {
		return err
	}
	to, err := os.OpenFile(destination, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, info.Mode())
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, to.Close()) }()
	if _, err := io.Copy(to, from); err != nil {
		return err
	}
	return to.Chmod(info.Mode())
}
