// Package storage holds a device's images and their generations
// (docs/cloud/managed-hosts.md § Images): the ext4 filesystems, copies of them,
// the immutable image store, the working pair and the import of a base.
package storage

import (
	"fmt"
	"os"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/internal/fsfail"
)

// WriteRecord replaces the file at path with data, synced before the rename
// (artifact's durable publication). The caller syncs the directory once the
// rename must survive a crash.
func WriteRecord(path string, data []byte) error {
	publication := artifact.Publication{
		Mode:        artifact.Replace,
		Permissions: artifact.DefaultPermissions,
		Durable:     true,
	}
	if err := artifact.PublishBytes(path, data, publication); err != nil {
		return fmt.Errorf("failed to write %s: %w", path, err)
	}
	return nil
}

// Sync puts a file's data or a directory's entries on disk.
func Sync(path string) error {
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	if err := file.Sync(); err != nil {
		file.Close()
		return err
	}
	return file.Close()
}

// RemoveTree removes the directory tree at path; one that is gone already is
// fine.
func RemoveTree(path string) error {
	return os.RemoveAll(path)
}

// CreatePrivate creates the directory path and any missing parents, each
// private to the owner; a directory that exists keeps its mode.
func CreatePrivate(path string) error {
	if err := os.MkdirAll(path, 0o700); err != nil {
		return fmt.Errorf("failed to create %s: %w", path, fsfail.Cause(err))
	}
	return nil
}
