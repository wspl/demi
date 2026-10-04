//go:build darwin || linux

package jobs

import (
	"context"
	"errors"
	"net"
	"os"
	"path/filepath"
	"time"

	"github.com/wspl/demi/internal/runner/process"
	"golang.org/x/sys/unix"
)

// bindListener owns the private Unix socket and the liveness lock clients inspect.
func bindListener(ctx context.Context) (*Listener, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	directory, err := os.MkdirTemp("", "demi-")
	if err != nil {
		return nil, err
	}
	success := false
	defer func() {
		if !success {
			_ = os.RemoveAll(directory)
		}
	}()
	alive, err := os.Create(filepath.Join(directory, process.Alive))
	if err != nil {
		return nil, err
	}
	defer func() {
		if !success {
			_ = alive.Close()
		}
	}()
	if err = unix.Flock(int(alive.Fd()), unix.LOCK_EX|unix.LOCK_NB); err != nil {
		return nil, err
	}
	path := filepath.Join(directory, "ipc.sock")
	socket, err := net.ListenUnix("unix", &net.UnixAddr{Name: path, Net: "unix"})
	if err != nil {
		return nil, err
	}
	if err = os.Chmod(path, 0o600); err != nil {
		_ = socket.Close()
		return nil, err
	}
	success = true
	return &Listener{
		endpoint: path,
		close: func() error {
			return errors.Join(socket.Close(), alive.Close(), os.RemoveAll(directory))
		},
		accept: func(ctx context.Context) (net.Conn, error) {
			return acceptLocal(ctx, socket)
		},
	}, nil
}

func acceptLocal(ctx context.Context, socket *net.UnixListener) (net.Conn, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		_ = socket.SetDeadline(time.Now())
		close(done)
	})
	conn, err := socket.AcceptUnix()
	if !stop() {
		<-done
	}
	if reset := socket.SetDeadline(time.Time{}); err == nil && reset != nil {
		if conn != nil {
			_ = conn.Close()
		}
		return nil, reset
	}
	if ctx.Err() != nil {
		if conn != nil {
			_ = conn.Close()
		}
		return nil, ctx.Err()
	}
	return conn, err
}
