//go:build linux

package storage

import (
	"context"
	"errors"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/contract"
	"golang.org/x/sys/unix"
)

// WriteJSON replaces path with value's JSON, synced before rename. The caller
// syncs its directory when the rename must survive a crash. Values are contract
// types or ordered structs, never unordered object maps.
func WriteJSON(ctx context.Context, path string, value any) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	data, err := contract.EncodeJSON(value)
	if err != nil {
		return err
	}
	if err := artifacts.PublishBytes(
		context.WithoutCancel(ctx),
		path,
		data,
		artifacts.Publication{Mode: artifacts.Replace, Permissions: artifacts.Default, Durable: true},
	); err != nil {
		return fmt.Errorf("failed to write %s: %w", path, err)
	}
	return nil
}

// Sync syncs a file's data or a directory's entries to disk.
func Sync(ctx context.Context, path string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	// Read-only descriptor; Sync reports durability errors.
	defer func() { _ = file.Close() }()
	return file.Sync()
}

// RemoveTree removes a directory tree; one already gone is fine.
func RemoveTree(ctx context.Context, path string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	// RemoveAll also removes a non-directory, so require a directory first
	// (or a symlink, which is removed without following it).
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	if !info.IsDir() && info.Mode()&os.ModeSymlink == 0 {
		return &os.PathError{Op: "remove", Path: path, Err: unix.ENOTDIR}
	}
	return os.RemoveAll(path)
}

// CreatePrivate creates path and missing parents private to the owner.
// Existing directories retain their modes.
func CreatePrivate(ctx context.Context, path string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := os.MkdirAll(path, 0o700); err != nil {
		return fmt.Errorf("failed to create %s: %w", path, err)
	}
	return nil
}
