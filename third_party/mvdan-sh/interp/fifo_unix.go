// Copyright (c) 2026, the Demi contributors
// See LICENSE for licensing information

//go:build unix

package interp

import (
	"context"
	"errors"
	"io"
	"os"
	"runtime"
	"sync"

	"golang.org/x/sys/unix"
)

// openFIFO preserves the FIFO's blocking rendezvous, but cancellation supplies
// the missing peer. Once open, nonblocking IO and poll make both directions
// cancellable, including on Darwin where os.File disables FIFO polling.
func openFIFO(ctx context.Context, path string, flags int) (io.ReadWriteCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	opened := make(chan struct{})
	interrupted := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(interrupted)
		// The owned FIFO cannot be removed until open returns. Keep the rescue
		// peer alive until then, even if cancellation precedes the blocking open.
		peer, err := unix.Open(path, unix.O_RDWR|unix.O_NONBLOCK|unix.O_CLOEXEC, 0)
		<-opened
		if err == nil {
			unix.Close(peer)
		}
	})
	fd, err := unix.Open(path, flags|unix.O_CLOEXEC, 0)
	close(opened)
	if !stop() {
		<-interrupted
	}
	if ctx.Err() != nil {
		if err == nil {
			unix.Close(fd)
		}
		return nil, ctx.Err()
	}
	if err != nil {
		return nil, &os.PathError{Op: "open", Path: path, Err: err}
	}
	if err := unix.SetNonblock(fd, true); err != nil {
		unix.Close(fd)
		return nil, err
	}
	return ownedContextFile(ctx, fd)
}

// BorrowFile gives a child IO adapter a cancellable duplicate of a shell file.
// It never hands the original file to os/exec: File.Fd would disable polling on
// that file, leaving a later shell read impossible to cancel. Close releases
// only the duplicate, so a command that stops reading cannot close shell stdin.
func BorrowFile(ctx context.Context, file *os.File) (io.ReadWriteCloser, error) {
	raw, err := file.SyscallConn()
	if err != nil {
		return nil, err
	}
	fd := -1
	var duplicateErr error
	if err := raw.Control(func(original uintptr) {
		fd, duplicateErr = unix.FcntlInt(original, unix.F_DUPFD_CLOEXEC, 0)
	}); err != nil {
		return nil, err
	}
	if duplicateErr != nil {
		return nil, duplicateErr
	}
	if err := unix.SetNonblock(fd, true); err != nil {
		unix.Close(fd)
		return nil, err
	}
	return ownedContextFile(ctx, fd)
}

func ownedContextFile(ctx context.Context, fd int) (io.ReadWriteCloser, error) {
	wake, signal, err := os.Pipe()
	if err != nil {
		unix.Close(fd)
		return nil, err
	}
	f := &fifoFile{ctx: ctx, fd: fd, wake: wake, signal: signal, canceled: make(chan struct{})}
	f.stop = context.AfterFunc(ctx, func() {
		signal.Close()
		close(f.canceled)
	})
	return f, nil
}

// fifoFile owns a nonblocking FIFO and a cancellation pipe. Its owner closes it
// after IO has stopped; Close also interrupts a concurrent read or write.
type fifoFile struct {
	ctx      context.Context
	fd       int
	wake     *os.File
	signal   *os.File
	stop     func() bool
	canceled chan struct{}
	once     sync.Once
	mu       sync.Mutex
	closed   bool
	active   sync.WaitGroup
}

func (f *fifoFile) io(p []byte, writing bool) (int, error) {
	f.mu.Lock()
	if f.closed {
		f.mu.Unlock()
		return 0, os.ErrClosed
	}
	f.active.Add(1)
	f.mu.Unlock()
	defer f.active.Done()
	for {
		if err := f.ctx.Err(); err != nil {
			return 0, err
		}
		var n int
		var err error
		events := int16(unix.POLLIN)
		if writing {
			events = unix.POLLOUT
			n, err = unix.Write(f.fd, p)
		} else {
			n, err = unix.Read(f.fd, p)
		}
		if err == unix.EINTR {
			continue
		}
		if !errors.Is(err, unix.EAGAIN) {
			if n < 0 {
				n = 0
			}
			if !writing && n == 0 && err == nil {
				err = io.EOF
			}
			return n, err
		}
		polls := []unix.PollFd{{Fd: int32(f.fd), Events: events}, {Fd: int32(f.wake.Fd()), Events: unix.POLLIN}}
		// Darwin can miss the last writer closing a FIFO (also noted in
		// os/file_unix.go). Recheck read/EOF at most 10 ms later there;
		// cancellation still wakes poll immediately through the wake pipe.
		timeout := -1
		if runtime.GOOS == "darwin" || runtime.GOOS == "ios" {
			timeout = 10
		}
		if _, err := unix.Poll(polls, timeout); err != nil && err != unix.EINTR {
			return 0, err
		}
		if polls[1].Revents != 0 {
			if err := f.ctx.Err(); err != nil {
				return 0, err
			}
			return 0, os.ErrClosed
		}
	}
}
func (f *fifoFile) Read(p []byte) (int, error) { return f.io(p, false) }
func (f *fifoFile) Write(p []byte) (int, error) {
	total := 0
	for len(p) > 0 {
		n, err := f.io(p, true)
		total += n
		p = p[n:]
		if err != nil {
			return total, err
		}
	}
	return total, nil
}
func (f *fifoFile) Close() error {
	f.once.Do(func() {
		f.mu.Lock()
		f.closed = true
		f.mu.Unlock()
		if f.stop() {
			f.signal.Close()
		} else {
			<-f.canceled
		}
		f.active.Wait()
		unix.Close(f.fd)
		f.wake.Close()
	})
	return nil
}
