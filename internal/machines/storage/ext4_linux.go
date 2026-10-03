//go:build linux

package storage

import (
	"context"
	"encoding/binary"
	"fmt"
	"math"
	"os"
	"time"

	"github.com/wspl/demi/internal/machines/system"
	"golang.org/x/sys/unix"
)

// Capacity reads block count times block size from the primary ext4 superblock.
func Capacity(ctx context.Context, image string) (uint64, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	file, err := os.Open(image)
	if err != nil {
		return 0, err
	}
	defer func() { _ = file.Close() }() // Read-only superblock descriptor.
	var block [1024]byte
	if _, err := file.ReadAt(block[:], 1024); err != nil {
		return 0, err
	}
	logSize := binary.LittleEndian.Uint32(block[0x18:])
	if binary.LittleEndian.Uint16(block[0x38:]) != 0xef53 || logSize > 6 {
		return 0, &NotExt4Error{Path: image}
	}
	blocks := uint64(binary.LittleEndian.Uint32(block[4:]))
	if binary.LittleEndian.Uint32(block[0x60:])&0x80 != 0 {
		blocks |= uint64(binary.LittleEndian.Uint32(block[0x150:])) << 32
	}
	size := uint64(1024) << logSize
	if blocks == 0 || blocks > math.MaxUint64/size {
		return 0, &NotExt4Error{Path: image}
	}
	return blocks * size, nil
}

// MakeSystem creates an empty system filesystem. bytes must be nonzero.
// Overlay upper and work directories are made when it is first mounted.
func MakeSystem(ctx context.Context, tools *system.Tools, image string, bytes uint64) error {
	deadline := 60 * time.Second
	_, err := tools.Run(
		ctx,
		system.Mke2fs,
		[]string{"-q", "-t", "ext4", "-F", "-L", "system", image, kibibytes(bytes)},
		&deadline,
	)
	return err
}

// MakeHome creates a home filesystem populated from root, including ownership
// and modes. bytes must be nonzero.
func MakeHome(ctx context.Context, tools *system.Tools, root, image string, bytes uint64) error {
	deadline := 60 * time.Second
	_, err := tools.Run(
		ctx,
		system.Mke2fs,
		[]string{"-q", "-t", "ext4", "-F", "-L", "home", "-d", root, image, kibibytes(bytes)},
		&deadline,
	)
	return err
}

// GrowMounted grows the mounted ext4 filesystem to device's size, without a
// deadline. A failure caused by missing CAP_SYS_RESOURCE names the capability.
func GrowMounted(ctx context.Context, tools *system.Tools, device string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	_, failed := tools.Run(context.WithoutCancel(ctx), system.Resize2fs, []string{device}, nil)
	if failed == nil {
		return nil
	}
	held, err := unix.PrctlRetInt(unix.PR_CAPBSET_READ, unix.CAP_SYS_RESOURCE, 0, 0, 0)
	if err != nil {
		return err
	}
	if held == 0 {
		return &GrowthCapabilityError{Source: failed}
	}
	return failed
}

// Recover checks an unmounted image, completes interrupted growth to its file
// size, syncs it, and returns its actual capacity. Checks and resizes have no
// deadline and are allowed to finish once started, including on cancellation.
func Recover(ctx context.Context, tools *system.Tools, image string) (uint64, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	ctx = context.WithoutCancel(ctx)
	if err := checkExt4(ctx, tools, image, "-p"); err != nil {
		return 0, err
	}
	info, err := os.Stat(image)
	if err != nil {
		return 0, err
	}
	capacity, err := Capacity(ctx, image)
	if err != nil {
		return 0, err
	}
	if uint64(info.Size()) > capacity {
		if err := checkExt4(ctx, tools, image, "-pf"); err != nil {
			return 0, err
		}
		if _, err := tools.Run(ctx, system.Resize2fs, []string{image}, nil); err != nil {
			return 0, err
		}
	}
	if err := Sync(ctx, image); err != nil {
		return 0, err
	}
	return Capacity(ctx, image)
}

// kibibytes formats mke2fs's size argument, rounded up without overflowing.
func kibibytes(bytes uint64) string {
	size := bytes / 1024
	if bytes%1024 != 0 {
		size++
	}
	return fmt.Sprintf("%dk", size)
}

// checkExt4 accepts e2fsck's successful repair status as well as a clean image.
func checkExt4(ctx context.Context, tools *system.Tools, image, mode string) error {
	output, err := tools.Output(ctx, system.E2fsck, []string{mode, image}, nil)
	if err != nil {
		return err
	}
	_, err = system.Accept(system.E2fsck, output, []int{0, 1})
	return err
}
