//go:build linux

package network

import (
	"errors"
)

var (
	// ErrExhausted means every slot is in use.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrExhausted = errors.New("All Cloud network slots are in use")
	// ErrNoBackendAddress means the backend resolved to no IPv4 addresses.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrNoBackendAddress = errors.New("Backend has no reachable IPv4 address")
	// ErrLoopbackBackend means a backend address is loopback or in 0.0.0.0/8.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrLoopbackBackend = errors.New("Backend URL must be reachable from Cloud, not host loopback")
)
