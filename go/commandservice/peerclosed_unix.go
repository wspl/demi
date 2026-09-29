//go:build !windows

package commandservice

import (
	"errors"
	"syscall"
)

// closedByPeer reports whether err is what the operating system says of a
// connection whose peer closed it: a write into a pipe or socket that has no
// reader, and a reset.
func closedByPeer(err error) bool {
	return errors.Is(err, syscall.EPIPE) || errors.Is(err, syscall.ECONNRESET)
}
