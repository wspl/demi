package storage

import (
	"context"
	"encoding/binary"
	"errors"
	"math/bits"
	"os"
	"strconv"
	"time"

	"github.com/wspl/demi/go/machines/internal/tools"
)

// mke2fsDeadline is how long mke2fs may take: it makes an empty filesystem in
// well under a minute.
const mke2fsDeadline = 60 * time.Second

// The superblock's place and the fields the capacity needs.
const (
	superblockOffset = 1024
	superblockBytes  = 1024
	ext4Magic        = 0xEF53
	incompat64Bit    = 0x80
)

// ErrGrowthCapability means resize2fs failed, and the programs the manager starts
// cannot hold CAP_SYS_RESOURCE, without which the kernel grows no mounted ext4
// filesystem (docs/cloud/setup.md § Linux requirements).
var ErrGrowthCapability = errors.New("growing a mounted ext4 filesystem needs CAP_SYS_RESOURCE; this host's manager lacks it")

// A NotExt4Error means an image is not an ext4 image.
type NotExt4Error struct {
	Image string
}

func (e *NotExt4Error) Error() string { return e.Image + " is not an ext4 image" }

// A GrowthError is a refused growth: the failed resize2fs, and that the manager
// lacks CAP_SYS_RESOURCE.
type GrowthError struct {
	Resize error
}

func (e *GrowthError) Error() string { return ErrGrowthCapability.Error() }

func (e *GrowthError) Unwrap() []error { return []error{ErrGrowthCapability, e.Resize} }

// Capacity returns a filesystem's capacity: its block count times its block
// size, read from the primary superblock (the same numbers dumpe2fs -h prints).
func Capacity(image string) (uint64, error) {
	file, err := os.Open(image)
	if err != nil {
		return 0, err
	}
	defer file.Close()
	superblock := make([]byte, superblockBytes)
	if _, err := file.ReadAt(superblock, superblockOffset); err != nil {
		return 0, err
	}
	capacity, ok := parseCapacity(superblock)
	if !ok {
		return 0, &NotExt4Error{Image: image}
	}
	return capacity, nil
}

// parseCapacity reads a capacity from a superblock; it reports false for one
// that is not ext4's or holds no blocks.
func parseCapacity(superblock []byte) (uint64, bool) {
	u32 := func(offset int) uint32 {
		return binary.LittleEndian.Uint32(superblock[offset:])
	}
	magic := binary.LittleEndian.Uint16(superblock[0x38:])
	logBlockSize := u32(0x18)
	if magic != ext4Magic || logBlockSize > 6 {
		return 0, false
	}
	blocks := uint64(u32(0x04))
	if u32(0x60)&incompat64Bit != 0 {
		blocks |= uint64(u32(0x150)) << 32
	}
	blockSize := uint64(1024) << logBlockSize
	overflow, capacity := bits.Mul64(blocks, blockSize)
	if overflow != 0 || capacity == 0 {
		return 0, false
	}
	return capacity, true
}

// kibibytes is mke2fs's size argument: whole KiB, rounded up.
func kibibytes(bytes uint64) string {
	return strconv.FormatUint((bytes+1023)/1024, 10) + "k"
}

// MakeSystem makes an empty system filesystem; the overlay's upper and work
// directories are made when it is first mounted.
func MakeSystem(ctx context.Context, t *tools.Tools, image string, bytes uint64) error {
	args := []string{"-q", "-t", "ext4", "-F", "-L", "system", image, kibibytes(bytes)}
	_, err := t.Run(ctx, tools.Mke2fs, args, mke2fsDeadline)
	return err
}

// MakeHome makes a home filesystem whose root is populated from root, ownership
// and modes included.
func MakeHome(ctx context.Context, t *tools.Tools, root, image string, bytes uint64) error {
	args := []string{"-q", "-t", "ext4", "-F", "-L", "home", "-d", root, image, kibibytes(bytes)}
	_, err := t.Run(ctx, tools.Mke2fs, args, mke2fsDeadline)
	return err
}

// Recover checks an unmounted image and completes a growth that was interrupted
// after its file was extended, then syncs the image and returns its capacity. A
// check or a resize runs as long as it needs.
func Recover(ctx context.Context, t *tools.Tools, image string) (uint64, error) {
	if err := check(ctx, t, image, "-p"); err != nil {
		return 0, err
	}
	info, err := os.Stat(image)
	if err != nil {
		return 0, err
	}
	filesystem, err := Capacity(image)
	if err != nil {
		return 0, err
	}
	if uint64(info.Size()) > filesystem {
		if err := check(ctx, t, image, "-pf"); err != nil {
			return 0, err
		}
		if _, err := t.Run(ctx, tools.Resize2fs, []string{image}, 0); err != nil {
			return 0, err
		}
	}
	if err := Sync(image); err != nil {
		return 0, err
	}
	return Capacity(image)
}

// check runs e2fsck with mode; exit 1 means it corrected the filesystem.
func check(ctx context.Context, t *tools.Tools, image, mode string) error {
	output, err := t.Output(ctx, tools.E2fsck, []string{mode, image}, 0)
	if err != nil {
		return err
	}
	_, err = tools.Accept(tools.E2fsck, output, 0, 1)
	return err
}
