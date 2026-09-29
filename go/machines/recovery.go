//go:build linux

package machines

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/wspl/demi/go/machines/internal/sandbox"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machinesproto"
)

// Recovery of what a stopped manager left behind (docs/cloud/managed-hosts.md §
// Startup and recovery): staging directories are removed, every recorded sandbox
// is fenced, and every working pair is published. It runs at startup, on
// reconcile, and in the stop-post recovery.

// ErrSlot means a runtime record names a network slot outside the configured
// pool.
var ErrSlot = errors.New("Existing Cloud slot exceeds configured pool")

// fenceAndSave fences each recorded sandbox and saves each working pair. Every
// device operation has finished and none can start: the caller holds the whole
// admission gate, or no manager serves.
func fenceAndSave(ctx context.Context, core *Core) error {
	working := core.Config.Working()
	if err := storage.CreatePrivate(working); err != nil {
		return err
	}
	if err := storage.CreatePrivate(core.Runsc.Root()); err != nil {
		return err
	}
	entries, err := os.ReadDir(working)
	if err != nil {
		return err
	}
	var names []string
	for _, entry := range entries {
		name := entry.Name()
		if strings.HasPrefix(name, ".") {
			// A stage has replaced neither a working nor a committed pair.
			if err := storage.RemoveTree(filepath.Join(working, name)); err != nil {
				return err
			}
			continue
		}
		names = append(names, name)
	}
	slices.Sort(names)
	for _, name := range names {
		device, err := machinesproto.ParseDeviceID(name)
		if err != nil {
			return fmt.Errorf("a working pair is not named by a device id: %w", err)
		}
		pair := storage.NewWorkingPair(working, device)
		record, err := readRecord(pair.SandboxRecord())
		if err != nil {
			return err
		}
		if record != nil {
			if !core.Slots.Contains(record.Slot) {
				return ErrSlot
			}
			if err := sandbox.Recorded(&core.Host, *record).Close(ctx, pair); err != nil {
				return err
			}
		}
		if err := pair.Save(ctx, core.Tools, core.Store, device); err != nil {
			return err
		}
	}
	return nil
}

// readRecord reads a runtime record; it returns nil when there is none.
func readRecord(path string) (*sandbox.Record, error) {
	data, err := os.ReadFile(path)
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	record, err := sandbox.DecodeRecord(data)
	if err != nil {
		return nil, fmt.Errorf("%s is not a valid runtime record: %w", path, err)
	}
	return &record, nil
}
