package interp

import (
	"context"
	"io"
	"os"
	"sync"
	"time"
)

// contextFile interrupts FIFO I/O as well as its initial open rendezvous.
// Closing the wrapper removes and joins its cancellation callback.
type contextFile struct {
	*os.File
	close func() error
}

func (f *contextFile) FileHandle() *os.File { return f.File }
func (f *contextFile) Close() error         { return f.close() }

func openedContext(ctx context.Context, path string, flags int, mode os.FileMode) (io.ReadWriteCloser, error) {
	file, err := openContext(ctx, path, flags, mode)
	if err != nil {
		return nil, err
	}
	info, err := file.Stat()
	if err != nil {
		file.Close()
		return nil, err
	}
	if info.Mode()&os.ModeNamedPipe == 0 {
		return file, nil
	}
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		file.SetDeadline(time.Now())
		close(done)
	})
	return &contextFile{File: file, close: sync.OnceValue(func() error {
		if !stop() {
			<-done
		}
		return file.Close()
	})}, nil
}
