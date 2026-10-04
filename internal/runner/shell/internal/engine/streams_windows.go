package engine

import (
	"context"
	"io"
	"os"

	"github.com/wspl/demi/internal/cmdsdk"
	"golang.org/x/sys/windows"
)

// borrowedFile hides Fd so process.Command uses owned, interruptible IO adapters.
type borrowedFile struct{ *os.File }

func borrowFile(ctx context.Context, file *os.File) (io.ReadWriteCloser, error) {
	return cmdsdk.Retry(ctx, func() (io.ReadWriteCloser, error) {
		raw, err := file.SyscallConn()
		if err != nil {
			return nil, err
		}
		var handle windows.Handle
		var duplicateErr error
		if err := raw.Control(func(original uintptr) {
			process := windows.CurrentProcess()
			duplicateErr = windows.DuplicateHandle(
				process,
				windows.Handle(original),
				process,
				&handle,
				0,
				false,
				windows.DUPLICATE_SAME_ACCESS,
			)
		}); err != nil {
			return nil, err
		}
		if duplicateErr != nil {
			return nil, duplicateErr
		}
		return &borrowedFile{os.NewFile(uintptr(handle), file.Name())}, nil
	})
}

func restoreInputPolling(_ *os.File) {}
