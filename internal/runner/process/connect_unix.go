//go:build darwin || linux

package process

import (
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/commandsdk"
	"golang.org/x/sys/unix"
)

func connectLocal(ctx context.Context, endpoint string) (net.Conn, error) {
	if !filepath.IsAbs(endpoint) {
		return nil, fmt.Errorf("local socket path must be absolute")
	}
	var backoff commandsdk.Backoff
	for {
		connection, err := (&net.Dialer{}).DialContext(ctx, "unix", endpoint)
		if err == nil {
			return connection, nil
		}
		retry := errors.Is(err, unix.ECONNREFUSED) || errors.Is(err, unix.EAGAIN) || commandsdk.Exhausted(err)
		if !retry || !runnerMayLive(endpoint) {
			return nil, err
		}
		if err := backoff.Wait(ctx); err != nil {
			return nil, err
		}
	}
}

// runnerMayLive distinguishes a busy runner from an abandoned socket. An
// exhausted descriptor table cannot prove death and therefore waits as well.
func runnerMayLive(endpoint string) bool {
	file, err := os.Open(filepath.Join(filepath.Dir(endpoint), Alive))
	if err != nil {
		return commandsdk.Exhausted(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = file.Close()
	}()
	return errors.Is(unix.Flock(int(file.Fd()), unix.LOCK_SH|unix.LOCK_NB), unix.EWOULDBLOCK)
}
