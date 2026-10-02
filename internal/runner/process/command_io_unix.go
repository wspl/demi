//go:build darwin || linux

package process

import (
	"context"
	"errors"
	"os"
	"slices"

	"github.com/wspl/demi/internal/cmdsdk"
	"golang.org/x/sys/unix"
)

// cancellableStdio registers owned standard files with Go's poller. A plain
// os.NewFile of a blocking duplicate cannot interrupt Read by closing it.
// Restore runs only after all IO is joined and all pollable copies are closed:
// standard handles can share one underlying file description and its flags.
func cancellableStdio(ctx context.Context, stdio Stdio) (Stdio, func() error, error) {
	initial := stdio
	type handle struct {
		original *os.File
		fd       uintptr
		flags    int
	}
	originals := []handle{}
	adapted := []*os.File{}
	restore := func(closeOriginals bool) error {
		var result error
		for _, entry := range originals {
			_, err := unix.FcntlInt(entry.fd, unix.F_SETFL, entry.flags)
			result = errors.Join(result, err)
			if closeOriginals {
				result = errors.Join(result, entry.original.Close())
			}
		}
		return result
	}
	// A preparation failure has no IO workers: close partial duplicates and
	// restore shared flags before returning the original streams to final cleanup.
	failed := func(err error) (Stdio, func() error, error) {
		for _, file := range adapted {
			_ = file.Close()
		}
		return initial, func() error { return nil }, errors.Join(err, restore(false))
	}
	files := []*os.File{nil, nil, nil}
	files[0], _ = stdio.Stdin.(*os.File)
	files[1], _ = stdio.Stdout.(*os.File)
	files[2], _ = stdio.Stderr.(*os.File)
	handles := make([]handle, len(files))
	// Snapshot descriptors and flags before changing any. Calling File.Fd again
	// could itself switch a Go-managed shared file description back to blocking.
	for i, file := range files {
		if file == nil {
			continue
		}
		fd := file.Fd()
		flags, err := unix.FcntlInt(fd, unix.F_GETFL, 0)
		if err != nil {
			return failed(err)
		}
		handles[i] = handle{original: file, fd: fd, flags: flags}
	}
	for i, file := range files {
		if file == nil {
			continue
		}
		entry := handles[i]
		duplicate, err := cmdsdk.Retry(ctx, func() (int, error) { return unix.FcntlInt(entry.fd, unix.F_DUPFD_CLOEXEC, 0) })
		if err != nil {
			return failed(err)
		}
		if err := unix.SetNonblock(duplicate, true); err != nil {
			_ = unix.Close(duplicate) // Preparation failed before ownership transfer.
			return failed(err)
		}
		if !slices.ContainsFunc(originals, func(h handle) bool { return h.original == file }) {
			originals = append(originals, entry)
		}
		stream := os.NewFile(uintptr(duplicate), file.Name())
		adapted = append(adapted, stream)
		switch i {
		case 0:
			stdio.Stdin = stream
		case 1:
			stdio.Stdout = stream
		case 2:
			stdio.Stderr = stream
		}
	}
	return stdio, func() error { return restore(true) }, nil
}
