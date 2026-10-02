//go:build linux

package storage_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"syscall"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machines/system/systemtest"
	"golang.org/x/sys/unix"
)

// allocatedBlocks observes the filesystem's actual allocation of a copied image.
func allocatedBlocks(path string) (int64, error) {
	info, err := os.Stat(path)
	if err != nil {
		return 0, err
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		return 0, fmt.Errorf("no Linux stat for %s", path)
	}
	return stat.Blocks, nil
}

// Cost: sparse 64 MiB copy, typically <1 s; no processes.
func TestSparseCopyKeepsHolesAndEndData(t *testing.T) {
	directory := t.TempDir()
	source := filepath.Join(directory, "source")
	destination := filepath.Join(directory, "copy")
	file, err := os.Create(source)
	requireStorage(t, err)
	defer func() { _ = file.Close() }() // Explicit close below reports write errors.
	for _, test := range []struct {
		offset int64
		data   []byte
	}{
		{0, []byte("head")}, {8 << 20, make([]byte, 1<<20)}, {(64 << 20) - 4, []byte("tail")},
	} {
		_, err := file.WriteAt(test.data, test.offset)
		requireStorage(t, err)
	}
	requireStorage(t, file.Close())
	requireStorage(t, storage.CloneSparse(t.Context(), source, destination))
	info, err := os.Stat(destination)
	requireStorage(t, err)
	if info.Size() != 64<<20 {
		t.Fatal(info.Size())
	}
	blocks, err := allocatedBlocks(destination)
	requireStorage(t, err)
	// Reflinks may share the source's written-zero extents. The cp comparison
	// below is the design's allocation oracle on reflink-capable filesystems.
	var fs unix.Statfs_t
	requireStorage(t, unix.Statfs(directory, &fs))
	if fs.Type == unix.EXT4_SUPER_MAGIC && blocks*512 >= 1<<20 {
		t.Fatalf("allocated %d blocks", blocks)
	}
	copiedFile, err := os.Open(destination)
	requireStorage(t, err)
	defer func() { _ = copiedFile.Close() }() // Read-only copy.
	for _, test := range []struct {
		offset int64
		want   string
	}{{0, "head"}, {(64 << 20) - 4, "tail"}} {
		var data [4]byte
		_, err := copiedFile.ReadAt(data[:], test.offset)
		requireStorage(t, err)
		if string(data[:]) != test.want {
			t.Fatalf("at %d: %q", test.offset, data)
		}
	}
	if err := storage.CloneSparse(t.Context(), source, destination); !errors.Is(err, os.ErrExist) {
		t.Fatalf("existing destination: %v", err)
	}
}

// Cost: opt-in 3 x 512 MiB sparse filesystem mounts, mkfs and copy oracles;
// normally <10 s, with no sleeps or polling other than system-owned loop detach.
func TestCopyMatchesCPOnXFSBtrfsExt4(t *testing.T) {
	directory := t.TempDir()
	isolatedStorage(t, func(ctx context.Context) error {
		content := filepath.Join(directory, "content")
		if err := os.Mkdir(content, 0700); err != nil {
			return err
		}
		for i := 0; i < 8; i++ {
			if err := os.WriteFile(filepath.Join(content, fmt.Sprintf("file-%d", i)), bytes.Repeat([]byte{byte(i + 1)}, 300000+i*4096), 0600); err != nil {
				return err
			}
		}
		source := filepath.Join(directory, "source.ext4")
		if _, err := diskCommand(ctx, "mke2fs", "-q", "-t", "ext4", "-d", content, source, "96m"); err != nil {
			return err
		}
		file, err := os.OpenFile(source, os.O_WRONLY, 0)
		if err != nil {
			return err
		}
		if _, err := file.WriteAt(make([]byte, 2<<20), 40<<20); err != nil {
			return errors.Join(err, file.Close())
		}
		if _, err := file.WriteAt([]byte("tail"), (96<<20)-4); err != nil {
			return errors.Join(err, file.Close())
		}
		if err := file.Close(); err != nil {
			return err
		}
		for _, filesystem := range []string{"xfs", "btrfs", "ext4"} {
			if err := compareImageCopies(ctx, t, directory, source, filesystem); err != nil {
				return fmt.Errorf("%s: %w", filesystem, err)
			}
		}
		return nil
	})
}

// compareImageCopies checks bytes and allocation against cp on one disposable filesystem.
func compareImageCopies(ctx context.Context, t *testing.T, directory, source, filesystem string) (err error) {
	image := filepath.Join(directory, filesystem+".img")
	file, err := os.Create(image)
	if err != nil {
		return err
	}
	if err := file.Truncate(512 << 20); err != nil {
		return errors.Join(err, file.Close())
	}
	if err := file.Close(); err != nil {
		return err
	}
	device, err := system.Attach(ctx, image)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, device.Close()) }()
	force := "-f"
	if filesystem == "ext4" {
		force = "-F"
	}
	if _, err := diskCommand(ctx, "mkfs."+filesystem, "-q", force, device.Path()); err != nil {
		return err
	}
	target := filepath.Join(directory, filesystem)
	if err := os.Mkdir(target, 0700); err != nil {
		return err
	}
	// system exposes ext4 only; this test's host-filesystem matrix needs the
	// generic mount syscall for its XFS and btrfs allocation oracles.
	if err := unix.Mount(device.Path(), target, filesystem, 0, ""); err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, system.Unmount(context.WithoutCancel(ctx), target), systemtest.WaitLoopDetach(context.WithoutCancel(ctx), image))
	}()
	if err := device.Close(); err != nil {
		return err
	}
	attached, err := systemtest.LoopAttached(ctx, image)
	if err != nil {
		return err
	}
	if !attached {
		return errors.New("mounted loop device is not attached")
	}
	inside := filepath.Join(target, "source.ext4")
	ours := filepath.Join(target, "ours.ext4")
	theirs := filepath.Join(target, "theirs.ext4")
	if _, err := diskCommand(ctx, "cp", "--sparse=always", source, inside); err != nil {
		return err
	}
	if err := storage.CloneSparse(ctx, inside, ours); err != nil {
		return err
	}
	if _, err := diskCommand(ctx, "cp", "--reflink=auto", "--sparse=always", inside, theirs); err != nil {
		return err
	}
	root, err := os.Open(target)
	if err != nil {
		return err
	}
	synced := unix.Syncfs(int(root.Fd()))
	if err := errors.Join(synced, root.Close()); err != nil {
		return err
	}
	original, err := artifacts.DigestFile(ctx, inside, math.MaxUint64)
	if err != nil {
		return err
	}
	copied, err := artifacts.DigestFile(ctx, ours, math.MaxUint64)
	if err != nil {
		return err
	}
	if original != copied {
		return errors.New("copied content differs")
	}
	info, err := os.Stat(ours)
	if err != nil {
		return err
	}
	if info.Size() != 96<<20 {
		return fmt.Errorf("copied size %d", info.Size())
	}
	oursBlocks, err := allocatedBlocks(ours)
	if err != nil {
		return err
	}
	theirsBlocks, err := allocatedBlocks(theirs)
	if err != nil {
		return err
	}
	t.Logf("%s: ours %d blocks, cp %d blocks", filesystem, oursBlocks, theirsBlocks)
	if oursBlocks > theirsBlocks {
		return fmt.Errorf("allocated %d blocks, cp allocated %d", oursBlocks, theirsBlocks)
	}
	return nil
}
