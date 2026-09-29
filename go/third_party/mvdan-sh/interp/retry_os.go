package interp

import (
	"context"
	"io/fs"
	"os"
)

// openFile applies the job's allocation policy even to substitution FIFOs.
func openFile(ctx context.Context, path string, flags int, mode os.FileMode) (file *os.File, err error) {
	err = retryIO(ctx, func() error {
		var err error
		file, err = os.OpenFile(path, flags, mode)
		return err
	})
	return file, err
}

// pipeContext applies the job's allocation policy to internal wakeup pipes.
func pipeContext(ctx context.Context) (reader, writer *os.File, err error) {
	err = retryIO(ctx, func() error {
		var err error
		reader, writer, err = os.Pipe()
		return err
	})
	return reader, writer, err
}

// readDirectory applies the job's allocation policy to glob enumeration.
func readDirectory(ctx context.Context, path string) (entries []fs.DirEntry, err error) {
	err = retryIO(ctx, func() error {
		var err error
		entries, err = os.ReadDir(path)
		return err
	})
	return entries, err
}
