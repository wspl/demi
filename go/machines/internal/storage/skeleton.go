//go:build linux

package storage

import (
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
)

// CopySkeleton copies the directory skeleton to the new directory destination
// (docs/cloud/managed-hosts.md § Container initialization): symbolic links as
// they are, and every entry owned by the sandbox's user.
func CopySkeleton(skeleton, destination string) error {
	return filepath.WalkDir(skeleton, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		relative, err := filepath.Rel(skeleton, path)
		if err != nil {
			return err
		}
		target := filepath.Join(destination, relative)
		info, err := entry.Info()
		if err != nil {
			return err
		}
		switch kind := info.Mode().Type(); {
		case kind.IsDir():
			if err := os.Mkdir(target, 0o777); err != nil {
				return err
			}
			if err := os.Chmod(target, info.Mode()); err != nil {
				return err
			}
		case kind == fs.ModeSymlink:
			link, err := os.Readlink(path)
			if err != nil {
				return err
			}
			if err := os.Symlink(link, target); err != nil {
				return err
			}
		case kind.IsRegular():
			if err := copyFile(path, target, info.Mode()); err != nil {
				return err
			}
		default:
			return fmt.Errorf("%s is neither a file, a directory nor a link", path)
		}
		return os.Lchown(target, UserID, UserID)
	})
}

// copyFile copies a file's contents and gives the copy the file's exact mode.
func copyFile(source, target string, mode fs.FileMode) error {
	from, err := os.Open(source)
	if err != nil {
		return err
	}
	defer from.Close()
	to, err := os.OpenFile(target, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, mode)
	if err != nil {
		return err
	}
	if _, err := io.Copy(to, from); err != nil {
		to.Close()
		return err
	}
	if err := to.Chmod(mode); err != nil {
		to.Close()
		return err
	}
	return to.Close()
}
