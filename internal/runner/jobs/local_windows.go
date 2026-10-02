package jobs

import (
	"context"
	"errors"
	"net"
	"os"
	"sync"

	"golang.org/x/sys/windows"
)

// bindLocal owns successive instances of a private, local-only named pipe.
func bindLocal(ctx context.Context) (*Listener, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	endpoint := `\\.\pipe\demi-` + executionID()
	name, err := windows.UTF16PtrFromString(endpoint)
	if err != nil {
		return nil, err
	}
	create := func(first bool) (windows.Handle, error) {
		flags := uint32(windows.PIPE_ACCESS_DUPLEX | windows.FILE_FLAG_OVERLAPPED)
		if first {
			flags |= windows.FILE_FLAG_FIRST_PIPE_INSTANCE
		}
		return windows.CreateNamedPipe(name, flags, windows.PIPE_TYPE_BYTE|windows.PIPE_READMODE_BYTE|windows.PIPE_WAIT|windows.PIPE_REJECT_REMOTE_CLIENTS, windows.PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, nil)
	}
	handle, err := create(true)
	if err != nil {
		return nil, err
	}
	var mu sync.Mutex // protects the listener handle, never a pipe wait.
	closed := false
	return &Listener{endpoint: endpoint, close: func() error {
		mu.Lock()
		closed = true
		owned := handle
		handle = windows.InvalidHandle
		mu.Unlock()
		return windows.CloseHandle(owned)
	}, accept: func(ctx context.Context) (net.Conn, error) {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		mu.Lock()
		current := handle
		mu.Unlock()
		event, err := windows.CreateEvent(nil, 1, 0, nil)
		if err != nil {
			return nil, err
		}
		defer func() { _ = windows.CloseHandle(event) }() // The event is private to this completed accept.
		overlapped := windows.Overlapped{HEvent: event}
		err = windows.ConnectNamedPipe(current, &overlapped)
		if errors.Is(err, windows.ERROR_IO_PENDING) {
			done := make(chan struct{})
			stop := context.AfterFunc(ctx, func() { _ = windows.CancelIoEx(current, &overlapped); close(done) })
			var transferred uint32
			err = windows.GetOverlappedResult(current, &overlapped, &transferred, true)
			if !stop() {
				<-done
			}
		}
		if errors.Is(err, windows.ERROR_PIPE_CONNECTED) {
			err = nil
		}
		if err != nil {
			return nil, err
		}
		next, err := create(false)
		if err != nil {
			return nil, err
		}
		mu.Lock()
		if closed {
			mu.Unlock()
			_ = windows.CloseHandle(next)
			return nil, net.ErrClosed
		}
		handle = next
		mu.Unlock()
		return &acceptedPipe{File: os.NewFile(uintptr(current), endpoint)}, nil
	}}, nil
}

// acceptedPipe adds the net.Conn addresses to an overlapped named-pipe file.
// cmdsdk.PipeConn requires distinct input/output ownership; this is one handle.
type acceptedPipe struct{ *os.File }

func (*acceptedPipe) LocalAddr() net.Addr  { return &net.UnixAddr{Name: "local", Net: "pipe"} }
func (*acceptedPipe) RemoteAddr() net.Addr { return &net.UnixAddr{Name: "peer", Net: "pipe"} }
