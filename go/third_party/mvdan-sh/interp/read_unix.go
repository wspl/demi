//go:build unix

package interp

import (
	"context"
	"golang.org/x/sys/unix"
	"io"
	"os"
)

// cancellableReader also works after os/exec has made the inherited descriptor
// blocking. Poll a dedicated cancellation pipe, without consuming shell input
// ahead of a read command or leaving a blocked read worker behind.
func cancellableReader(ctx context.Context, file *os.File) (func([]byte) (int, error), func(), error) {
	wake, signal, err := pipeContext(ctx)
	if err != nil {
		return nil, nil, err
	}
	raw, err := file.SyscallConn()
	if err != nil {
		wake.Close()
		signal.Close()
		return nil, nil, err
	}
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		signal.Close()
		close(done)
	})
	cleanup := func() {
		if !stop() {
			<-done
		}
		signal.Close()
		wake.Close()
	}
	wakeFD := int(wake.Fd())
	read := func(p []byte) (int, error) {
		if err := ctx.Err(); err != nil {
			return 0, err
		}
		var n int
		var readErr error
		err := raw.Control(func(fd uintptr) {
			if readErr = unix.SetNonblock(int(fd), true); readErr != nil {
				return
			}
			poll := []unix.PollFd{{Fd: int32(fd), Events: unix.POLLIN}, {Fd: int32(wakeFD), Events: unix.POLLIN}}
			for {
				if err := ctx.Err(); err != nil {
					readErr = err
					return
				}
				_, readErr = unix.Poll(poll, -1)
				if readErr == unix.EINTR {
					continue
				}
				if readErr != nil {
					return
				}
				if err := ctx.Err(); err != nil {
					readErr = err
					return
				}
				n, readErr = unix.Read(int(fd), p)
				if readErr == unix.EAGAIN || readErr == unix.EINTR {
					continue
				}
				return
			}
		})
		if err != nil {
			return 0, err
		}
		if n == 0 && readErr == nil {
			readErr = io.EOF
		}
		return n, readErr
	}
	return read, cleanup, nil
}
