//go:build linux

package system

import "fmt"

// Failed describes a failed system call on path, preserving its cause for errors.Is/As.
func Failed(action, path string, err error) error {
	if err == nil {
		return nil
	}
	return fmt.Errorf("%s %s: %w", action, path, err)
}
