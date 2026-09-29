//go:build unix

package interp

import (
	"context"
	"golang.org/x/sys/unix"
	"os"
)

// openContext releases a blocked FIFO rendezvous when the job is canceled.
// Ordinary files use the synchronous open path and do not allocate a worker.
func openContext(ctx context.Context, path string, flags int, mode os.FileMode) (*os.File, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	info, err := os.Stat(path)
	if err != nil || info.Mode()&os.ModeNamedPipe == 0 {
		return openFile(ctx, path, flags, mode)
	}
	type result struct {
		file *os.File
		err  error
	}
	done := make(chan result, 1)
	go func() {
		f, err := openFile(ctx, path, flags, mode)
		done <- result{f, err}
	}()
	select {
	case got := <-done:
		return got.file, got.err
	case <-ctx.Done():
		// If the opener itself was waiting for a descriptor, it can finish
		// on cancellation without needing a peer. Check that on every retry:
		// cleanup must not wait for a descriptor after the opener has ended.
		var got result
		var joined bool
		var peer *os.File
		err := retryIO(context.WithoutCancel(ctx), func() error {
			select {
			case got = <-done:
				joined = true
				return nil
			default:
			}
			// Opening both ends wakes either kind of blocked FIFO open.
			var err error
			peer, err = os.OpenFile(path, os.O_RDWR|unix.O_NONBLOCK, 0)
			return err
		})
		if err != nil {
			return nil, err
		}
		if !joined {
			got = <-done
		}
		if peer != nil {
			peer.Close()
		}
		if got.file != nil {
			got.file.Close()
		}
		return nil, ctx.Err()
	}
}
