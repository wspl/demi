//go:build linux

// Package linux holds the kernel interfaces the machine manager uses instead of
// administration tools (docs/cloud/managed-hosts.md § Linux control): mounts,
// loop devices, filesystem freezes and work inside another namespace.
package linux

import "fmt"

// Failed returns err, the failure of the system call action on path, with both
// named; errors.Is still finds the errno.
func Failed(action, path string, err error) error {
	return fmt.Errorf("%s %s: %w", action, path, err)
}
