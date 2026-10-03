package process

import (
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"strings"
	"time"

	"golang.org/x/sys/windows"
)

func connectLocal(ctx context.Context, endpoint string) (net.Conn, error) {
	if !strings.HasPrefix(endpoint, `\\.\pipe\demi-`) {
		return nil, fmt.Errorf("invalid local named pipe endpoint")
	}
	name, err := windows.UTF16PtrFromString(endpoint)
	if err != nil {
		return nil, err
	}
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		handle, err := windows.CreateFile(
			name,
			windows.GENERIC_READ|windows.GENERIC_WRITE,
			0,
			nil,
			windows.OPEN_EXISTING,
			windows.FILE_FLAG_OVERLAPPED,
			0,
		)
		if err == nil {
			return &localPipe{File: os.NewFile(uintptr(handle), endpoint)}, nil
		}
		if !errors.Is(err, windows.ERROR_PIPE_BUSY) {
			return nil, err
		}
		timer := time.NewTimer(10 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil, ctx.Err()
		case <-timer.C:
		}
	}
}

type localPipe struct{ *os.File }

func (*localPipe) LocalAddr() net.Addr  { return &net.UnixAddr{Name: "local", Net: "pipe"} }
func (*localPipe) RemoteAddr() net.Addr { return &net.UnixAddr{Name: "peer", Net: "pipe"} }
