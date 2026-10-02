//go:build darwin || linux

package engine

import (
	"context"
	"io"
	"os"

	"github.com/wspl/demi/internal/cmdsdk"
	"golang.org/x/sys/unix"
	"mvdan.cc/sh/v3/interp"
)

func borrowFile(ctx context.Context, file *os.File) (io.ReadWriteCloser, error) {
	return cmdsdk.Retry(ctx, func() (io.ReadWriteCloser, error) { return interp.BorrowFile(ctx, file) })
}

// restoreInputPolling reverses File.Fd's blocking mode after child inheritance.
// The shell keeps the descriptor and must be able to cancel its next read.
func restoreInputPolling(file *os.File) {
	info, err := file.Stat()
	if err != nil || info.Mode()&os.ModeNamedPipe == 0 {
		return
	}
	raw, err := file.SyscallConn()
	if err != nil {
		return
	}
	_ = raw.Control(func(fd uintptr) { _ = unix.SetNonblock(int(fd), true) })
} // Cancellation may already have closed the invocation-owned file.
