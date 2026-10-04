//go:build darwin || linux

package jobs_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/wspl/demi/internal/commandsdk/commandsdktest"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/process"
	"golang.org/x/sys/unix"
)

func TestClientWaitsForBusyRunnerButNotGoneRunner(t *testing.T) {
	ctx := t.Context()
	listener, err := jobs.BindListener(ctx)
	if err != nil {
		t.Fatal(err)
	}
	endpoint := listener.Endpoint()
	if err = listener.Close(); err != nil {
		t.Fatal(err)
	}
	if conn, err := process.Connect(ctx, endpoint); err == nil {
		_ = conn.Close()
		t.Fatal("connected to stopped runner")
	}
	directory, err := os.MkdirTemp("", "demi-busy-")
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = os.RemoveAll(directory) }()
	endpoint = filepath.Join(directory, "ipc.sock")
	fd, err := unix.Socket(unix.AF_UNIX, unix.SOCK_STREAM, 0)
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = unix.Close(fd) }()
	if err = unix.Bind(fd, &unix.SockaddrUnix{Name: endpoint}); err != nil {
		t.Fatal(err)
	}
	alive, err := os.Create(filepath.Join(directory, process.Alive))
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = alive.Close() }()
	if conn, err := process.Connect(ctx, endpoint); err == nil {
		_ = conn.Close()
		t.Fatal("connected to crashed runner")
	}
	if err = unix.Flock(int(alive.Fd()), unix.LOCK_EX|unix.LOCK_NB); err != nil {
		t.Fatal(err)
	}
	if err = unix.Listen(fd, 1); err != nil {
		t.Fatal(err)
	}
	var queued []int
	defer func() {
		for _, socket := range queued {
			_ = unix.Close(socket)
		}
	}()
	for {
		socket, err := unix.Socket(unix.AF_UNIX, unix.SOCK_STREAM, 0)
		if err != nil {
			t.Fatal(err)
		}
		queued = append(queued, socket)
		if err = unix.SetNonblock(socket, true); err != nil {
			t.Fatal(err)
		}
		err = unix.Connect(socket, &unix.SockaddrUnix{Name: endpoint})
		if err != nil {
			if !errors.Is(err, unix.EAGAIN) && !errors.Is(err, unix.EINPROGRESS) && !errors.Is(err, unix.ECONNREFUSED) {
				t.Fatal(err)
			}
			break
		}
	}
	baseline := commandsdktest.Pauses()
	done := make(chan error, 1)
	waiting, cancel := context.WithCancel(ctx)
	defer cancel()
	go func() {
		conn, err := process.Connect(waiting, endpoint)
		if conn != nil {
			_ = conn.Close()
		}
		done <- err
	}()
	// The pause counter offers no event, so the loop yields between checks.
	for commandsdktest.Pauses() == baseline {
		select {
		case err := <-done:
			t.Fatalf("busy runner refused caller: %v", err)
		default:
			runtime.Gosched()
		}
	}
	if err = unix.Flock(int(alive.Fd()), unix.LOCK_UN); err != nil {
		t.Fatal(err)
	}
	if err = unix.Shutdown(fd, unix.SHUT_RDWR); err != nil && !errors.Is(err, unix.ENOTCONN) {
		t.Fatal(err)
	}
	// Remove the public endpoint so the already-waiting client observes the runner's departure.
	if err = os.Remove(endpoint); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err == nil {
		t.Fatal("connected after runner departed")
	}
}
