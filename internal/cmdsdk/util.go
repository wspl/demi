package cmdsdk

import (
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"time"
)

// CommandService is the sole argument used to launch native command services.
const CommandService = "--command-service"

// Launch is the validated native service launch mode.
type Launch struct{}

// ParseLaunch accepts only --command-service, without help or version flags.
func ParseLaunch(args []string) (Launch, error) {
	if len(args) != 1 || args[0] != CommandService {
		return Launch{}, fmt.Errorf("usage: %s", CommandService)
	}
	return Launch{}, nil
}

// Resolve resolves a path against invocation cwd without collapsing symlink-sensitive '..'.
func Resolve(cwd, path string) (string, error) {
	if path == "" || strings.ContainsRune(path, 0) {
		return "", errors.New("path must be nonempty and contain no NUL byte")
	}
	if filepath.IsAbs(path) {
		return path, nil
	}
	if runtime.GOOS == "windows" {
		if filepath.VolumeName(path) != "" {
			return path, nil
		}
		if path[0] == '\\' || path[0] == '/' {
			return filepath.VolumeName(cwd) + path, nil
		}
	}
	if cwd == "" {
		return path, nil
	}
	return strings.TrimRight(cwd, string(os.PathSeparator)) + string(os.PathSeparator) + path, nil
}

var pauses atomic.Uint64

// DescriptorPauses exposes retry observations for cmdsdktest.Pauses.
func DescriptorPauses() uint64 { return pauses.Load() }

// Backoff spaces descriptor retries from 5 ms up to 100 ms; its zero value is ready.
type Backoff struct{ delay time.Duration }

// Pause returns and advances the next delay.
func (b *Backoff) Pause() time.Duration {
	if b.delay == 0 {
		b.delay = 5 * time.Millisecond
	}
	d := b.delay
	b.delay = min(d*2, 100*time.Millisecond)
	pauses.Add(1)
	return d
}

// Wait waits for the next descriptor retry or cancellation.
func (b *Backoff) Wait(ctx context.Context) error {
	timer := time.NewTimer(b.Pause())
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}

// Retry waits out descriptor exhaustion. Cancellation returns the last open-file error.
func Retry[T any](ctx context.Context, attempt func() (T, error)) (T, error) {
	var b Backoff
	for {
		v, err := attempt()
		if !Exhausted(err) {
			return v, err
		}
		if b.Wait(ctx) != nil {
			return v, err
		}
	}
}

// PipeConn adapts owned input/output files to a duplex HTTP/2 transport.
// Close releases both files; callers must not use them afterward.
type PipeConn struct{ Reader, Writer *os.File }

func (c *PipeConn) Read(b []byte) (int, error)  { return c.Reader.Read(b) }
func (c *PipeConn) Write(b []byte) (int, error) { return c.Writer.Write(b) }

// Close releases both pipe ends.
func (c *PipeConn) Close() error { return errors.Join(c.Reader.Close(), c.Writer.Close()) }

// LocalAddr identifies the local pipe end.
func (c *PipeConn) LocalAddr() net.Addr { return &net.UnixAddr{Name: "local", Net: "pipe"} }

// RemoteAddr identifies the peer pipe end.
func (c *PipeConn) RemoteAddr() net.Addr { return &net.UnixAddr{Name: "peer", Net: "pipe"} }

// SetDeadline sets both pipe deadlines.
func (c *PipeConn) SetDeadline(t time.Time) error {
	return errors.Join(c.SetReadDeadline(t), c.SetWriteDeadline(t))
}

// SetReadDeadline sets the input pipe deadline.
func (c *PipeConn) SetReadDeadline(t time.Time) error { return c.Reader.SetReadDeadline(t) }

// SetWriteDeadline sets the output pipe deadline.
func (c *PipeConn) SetWriteDeadline(t time.Time) error { return c.Writer.SetWriteDeadline(t) }
