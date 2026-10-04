//go:build linux

package storage

import (
	"bytes"
	"context"
	"errors"
	"os"

	"github.com/wspl/demi/internal/machinemanager/system"
	"golang.org/x/sys/unix"
)

// CloneSparse copies an image using a reflink when available, otherwise copying
// data ranges while preserving holes and skipping zero blocks. It creates a
// new destination with mode 0600, preserving the source length and bytes.
func CloneSparse(ctx context.Context, source, destination string) (err error) {
	if err := ctx.Err(); err != nil {
		return err
	}
	from, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() {
		_ = from.Close()
	}() // Read-only source.
	to, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, to.Close())
	}()
	err = unix.IoctlFileClone(int(to.Fd()), int(from.Fd()))
	if err == nil {
		return nil
	}
	if !errors.Is(err, unix.EOPNOTSUPP) && !errors.Is(err, unix.EXDEV) && !errors.Is(err, unix.EINVAL) &&
		!errors.Is(err, unix.ENOTTY) &&
		!errors.Is(err, unix.ENOSYS) {
		return system.Failed("cloning to", destination, err)
	}
	return copySparseRanges(ctx, from, to)
}

// writeNonzero copies runs of nonzero filesystem blocks in a sparse image.
func writeNonzero(ctx context.Context, to *os.File, chunk []byte, position int64) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	var zero [4096]byte
	run := -1
	for start := 0; start < len(chunk); start += len(zero) {
		block := chunk[start:min(start+len(zero), len(chunk))]
		if !bytes.Equal(block, zero[:len(block)]) {
			if run == -1 {
				run = start
			}
		} else if run != -1 {
			if _, err := to.WriteAt(chunk[run:start], position+int64(run)); err != nil {
				return err
			}
			run = -1
		}
	}
	if run != -1 {
		_, err := to.WriteAt(chunk[run:], position+int64(run))
		return err
	}
	return nil
}

// copySparseRanges copies allocated ranges and restores the source length, including trailing holes.
func copySparseRanges(ctx context.Context, from, to *os.File) error {
	info, err := from.Stat()
	if err != nil {
		return err
	}
	buffer := make([]byte, 1<<20)
	for offset := int64(0); offset < info.Size(); {
		if err := ctx.Err(); err != nil {
			return err
		}
		data, err := unix.Seek(int(from.Fd()), offset, unix.SEEK_DATA)
		if errors.Is(err, unix.ENXIO) {
			break
		}
		if err != nil {
			return err
		}
		hole, err := unix.Seek(int(from.Fd()), data, unix.SEEK_HOLE)
		if err != nil {
			return err
		}
		for position := data; position < hole; {
			chunk := buffer[:min(int64(len(buffer)), hole-position)]
			if _, err := from.ReadAt(chunk, position); err != nil {
				return err
			}
			if err := writeNonzero(ctx, to, chunk, position); err != nil {
				return err
			}
			position += int64(len(chunk))
		}
		offset = hole
	}
	return to.Truncate(info.Size())
}
