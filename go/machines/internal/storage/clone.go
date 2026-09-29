//go:build linux

package storage

import (
	"bytes"
	"errors"
	"os"

	"golang.org/x/sys/unix"

	"github.com/wspl/demi/go/machines/internal/linux"
)

// blockBytes is the granularity of the zero-block check: a filesystem block.
const blockBytes = 4096

// chunkBytes is how much of a data range is read at once.
const chunkBytes = 1 << 20

// CloneSparse copies source to the new file destination, mode 0600, keeping its
// length and every byte it holds (docs/cloud/managed-hosts.md § Images): a
// reflink clone where the filesystem supports one, and otherwise a copy of only
// the ranges that hold data, skipping holes and all-zero blocks, so unused
// capacity never becomes allocated host storage. It runs in the manager's
// process, so the checkpoint's frozen window starts no program.
func CloneSparse(source, destination string) error {
	from, err := os.Open(source)
	if err != nil {
		return err
	}
	defer from.Close()
	to, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	if err := copyImage(from, to, destination); err != nil {
		to.Close()
		return err
	}
	return to.Close()
}

func copyImage(from, to *os.File, destination string) error {
	err := unix.IoctlFileClone(int(to.Fd()), int(from.Fd()))
	switch {
	case err == nil:
		return nil
	case errors.Is(err, unix.EOPNOTSUPP), errors.Is(err, unix.EXDEV), errors.Is(err, unix.EINVAL),
		errors.Is(err, unix.ENOTTY), errors.Is(err, unix.ENOSYS):
		// The filesystem cannot share extents between these files.
	default:
		return linux.Failed("cloning to", destination, err)
	}
	info, err := from.Stat()
	if err != nil {
		return err
	}
	length := info.Size()
	if err := copyData(from, to, length); err != nil {
		return err
	}
	return to.Truncate(length)
}

// copyData copies each data range of from, leaving holes and all-zero blocks
// unwritten.
func copyData(from, to *os.File, length int64) error {
	buffer := make([]byte, chunkBytes)
	for offset := int64(0); offset < length; {
		data, err := unix.Seek(int(from.Fd()), offset, unix.SEEK_DATA)
		if errors.Is(err, unix.ENXIO) {
			// No data after offset: the rest is a hole.
			return nil
		}
		if err != nil {
			return err
		}
		hole, err := unix.Seek(int(from.Fd()), data, unix.SEEK_HOLE)
		if err != nil {
			return err
		}
		for position := data; position < hole; {
			chunk := buffer[:min(hole-position, chunkBytes)]
			if _, err := from.ReadAt(chunk, position); err != nil {
				return err
			}
			if err := writeNonZero(to, chunk, position); err != nil {
				return err
			}
			position += int64(len(chunk))
		}
		offset = hole
	}
	return nil
}

// writeNonZero writes the runs of chunk's blocks that are not all zero.
func writeNonZero(to *os.File, chunk []byte, position int64) error {
	zero := make([]byte, blockBytes)
	run := -1
	for start := 0; start < len(chunk); start += blockBytes {
		block := chunk[start:min(start+blockBytes, len(chunk))]
		isZero := bytes.Equal(block, zero[:len(block)])
		switch {
		case !isZero && run < 0:
			run = start
		case isZero && run >= 0:
			if _, err := to.WriteAt(chunk[run:start], position+int64(run)); err != nil {
				return err
			}
			run = -1
		}
	}
	if run >= 0 {
		if _, err := to.WriteAt(chunk[run:], position+int64(run)); err != nil {
			return err
		}
	}
	return nil
}
