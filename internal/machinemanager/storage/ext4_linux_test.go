//go:build linux

package storage_test

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/storage"
	"github.com/wspl/demi/internal/machinemanager/system"
	"github.com/wspl/demi/internal/machinemanager/system/systemtest"
	"golang.org/x/sys/unix"
)

// Cost: small local superblock fixtures; no subprocesses.
func TestCapacityIsBlocksTimesBlockSize(t *testing.T) {
	cases := []struct {
		name                  string
		low, high, log, flags uint32
		badMagic              bool
		want                  uint64
	}{
		{name: "ordinary", low: 262144, log: 2, want: 1 << 30},
		{name: "high-ignored", high: 1, log: 2},
		{name: "64-bit", high: 1, log: 2, flags: 0x80, want: (1 << 32) * 4096},
		{name: "bad-magic", low: 1, log: 2, badMagic: true},
		{name: "bad-block-size", low: 1, log: 7},
		{name: "overflow", low: 0xffffffff, high: 0xffffffff, log: 6, flags: 0x80},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			var data [2048]byte
			block := data[1024:]
			binary.LittleEndian.PutUint32(block[4:], test.low)
			binary.LittleEndian.PutUint32(block[0x150:], test.high)
			binary.LittleEndian.PutUint32(block[0x18:], test.log)
			binary.LittleEndian.PutUint32(block[0x60:], test.flags)
			if !test.badMagic {
				binary.LittleEndian.PutUint16(block[0x38:], 0xef53)
			}
			path := filepath.Join(t.TempDir(), "image")
			requireStorage(t, os.WriteFile(path, data[:], 0o600))
			got, err := storage.Capacity(t.Context(), path)
			if test.want == 0 {
				if err == nil || err.Error() != path+" is not an ext4 image" {
					t.Fatalf("invalid image accepted: %d, %v", got, err)
				}
			} else if err != nil || got != test.want {
				t.Fatalf("capacity = %d, %v; want %d", got, err, test.want)
			}
		})
	}
}

// Cost: mke2fs + dumpe2fs on a sparse 48 MiB image; normally <1 s.
func TestCapacityMatchesDumpe2fs(t *testing.T) {
	image := filepath.Join(t.TempDir(), "system.ext4")
	requireStorage(t, storage.MakeSystem(t.Context(), storageTools(t), image, 48<<20))
	printed, err := diskCommand(t.Context(), "dumpe2fs", "-h", image)
	requireStorage(t, err)
	fields := make(map[string]uint64)
	for _, line := range strings.Split(string(printed), "\n") {
		name, value, ok := strings.Cut(line, ":")
		if ok && (name == "Block count" || name == "Block size") {
			fields[name], err = strconv.ParseUint(strings.TrimSpace(value), 10, 64)
			requireStorage(t, err)
		}
	}
	expected := fields["Block count"] * fields["Block size"]
	got, err := storage.Capacity(t.Context(), image)
	requireStorage(t, err)
	if got != expected || got != 48<<20 {
		t.Fatalf("got %d, dumpe2fs %d", got, expected)
	}
}

// Cost: ext4 creation, two checks and resize of a sparse 64 MiB image; <1 s typical.
func TestRecoveryCompletesInterruptedGrowth(t *testing.T) {
	ctx := t.Context()
	tools := storageTools(t)
	root := filepath.Join(t.TempDir(), "mkhome")
	requireStorage(t, os.MkdirAll(filepath.Join(root, "demi/work"), 0o755))
	requireStorage(t, os.WriteFile(filepath.Join(root, "demi/work/a.txt"), []byte("alpha\n"), 0o600))
	image := filepath.Join(t.TempDir(), "home.ext4")
	requireStorage(t, storage.MakeHome(ctx, tools, root, image, 64<<20))
	info, err := os.Stat(image)
	requireStorage(t, err)
	if info.Size() != 64<<20 {
		t.Fatal(info.Size())
	}
	listed, err := diskCommand(ctx, "debugfs", "-R", "cat /demi/work/a.txt", image)
	requireStorage(t, err)
	if string(listed) != "alpha\n" {
		t.Fatalf("home data: %q", listed)
	}
	requireStorage(t, os.Truncate(image, 128<<20))
	capacity, err := storage.Recover(ctx, tools, image)
	requireStorage(t, err)
	if capacity != 128<<20 {
		t.Fatal(capacity)
	}
	_, err = diskCommand(ctx, "e2fsck", "-fn", image)
	requireStorage(t, err)
	listed, err = diskCommand(ctx, "debugfs", "-R", "cat /demi/work/a.txt", image)
	requireStorage(t, err)
	if string(listed) != "alpha\n" {
		t.Fatalf("growth lost data: %q", listed)
	}
}

// Cost: opt-in isolated loop mount and refused online resize; normally <1 s.
func TestGrowthWithoutCapabilityNamesCapability(t *testing.T) {
	directory := t.TempDir()
	tools := storageTools(t)
	isolatedStorage(t, func(ctx context.Context) (err error) {
		// The namespace thread retires on return; no other test loses this capability.
		if err := unix.Prctl(unix.PR_CAPBSET_DROP, unix.CAP_SYS_RESOURCE, 0, 0, 0); err != nil {
			return err
		}
		image := filepath.Join(directory, "volume.ext4")
		if err := storage.MakeSystem(ctx, tools, image, 32<<20); err != nil {
			return err
		}
		target := filepath.Join(directory, "mount")
		if err := os.Mkdir(target, 0o700); err != nil {
			return err
		}
		device, err := system.Attach(ctx, image)
		if err != nil {
			return err
		}
		defer func() {
			err = errors.Join(err, device.Close())
		}()
		if err := system.Ext4(ctx, device.Path(), target); err != nil {
			return err
		}
		defer func() {
			err = errors.Join(
				err,
				system.Unmount(context.WithoutCancel(ctx), target),
				systemtest.WaitLoopDetach(context.WithoutCancel(ctx), image),
			)
		}()
		number := device.Number()
		if err := device.Close(); err != nil {
			return err
		}
		if err := os.Truncate(image, 64<<20); err != nil {
			return err
		}
		if err := system.RefreshCapacity(ctx, number); err != nil {
			return err
		}
		refused := storage.GrowMounted(ctx, tools, system.LoopPath(number))
		var capability *storage.GrowthCapabilityError
		if !errors.As(refused, &capability) ||
			refused.Error() != "growing a mounted ext4 filesystem needs CAP_SYS_RESOURCE; this host's manager lacks it" {
			return fmt.Errorf("resize error: %v", refused)
		}
		capacity, err := storage.Capacity(ctx, image)
		if err != nil {
			return err
		}
		if capacity != 32<<20 {
			return fmt.Errorf("refused growth changed capacity to %d", capacity)
		}
		return nil
	})
}
