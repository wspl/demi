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
	"github.com/wspl/demi/internal/machineproto"
)

// FenceAndSave fences recorded sandboxes and publishes every working pair.
// The caller holds exclusive admission, or has not begun serving requests.
func FenceAndSave(ctx context.Context, core *Core) error {
	if err := storage.CreatePrivate(ctx, core.Runsc.Root()); err != nil {
		return err
	}
	return fenceAndSave(ctx, core)
}

// fenceAndSave fences and publishes the recorded Cloud working pairs.
func fenceAndSave(ctx context.Context, core *Core) error {
	if err := storage.CreatePrivate(ctx, core.Config.Working()); err != nil {
		return err
	}
	entries, err := os.ReadDir(core.Config.Working())
	if err != nil {
		return err
	}
	names := make([]string, 0, len(entries))
	for _, entry := range entries {
		name := entry.Name()
		if strings.HasPrefix(name, ".") {
			if err = storage.RemoveTree(ctx, filepath.Join(core.Config.Working(), name)); err != nil {
				return err
			}
			continue
		}
		names = append(names, name)
	}
	for _, name := range names {
		device, err := machineproto.ParseDeviceID(name)
		if err != nil {
			return fmt.Errorf("a working pair is not named by a device id: %w", err)
		}
		pair := workingImages{storage.NewWorkingPair(core.Config.Working(), device)}
		data, err := os.ReadFile(pair.SandboxRecord())
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		if err == nil {
			record, err := sandbox.DecodeRecord(data)
			if err != nil {
				return fmt.Errorf("%s is not a valid runtime record: %w", pair.SandboxRecord(), err)
			}
			if !core.Slots.Contains(record.Slot) {
				//nolint:staticcheck // User-visible text, kept byte for byte.
				return errors.New("Existing Cloud slot exceeds configured pool")
			}
			runtime := sandbox.Recorded(
				core.sandboxConfig(),
				core.dependencies(),
				record,
				core.Slots.Slot(record.Slot).Namespace(),
			)
			if err = runtime.Close(ctx, pair); err != nil {
				return err
			}
		}
		if err = pair.Save(ctx, core.Tools, core.Store, device); err != nil {
			return err
		}
	}
	return nil
}
