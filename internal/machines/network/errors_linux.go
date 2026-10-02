//go:build linux

package network

import (
	"errors"
	"net/netip"
)

var (
	// ErrExhausted means every slot is in use.
	//nolint:staticcheck // Preserve the Rust user-facing error verbatim.
	ErrExhausted = errors.New("All Cloud network slots are in use")
	// ErrNoBackendAddress means the backend resolved to no IPv4 addresses.
	//nolint:staticcheck // Preserve the Rust user-facing error verbatim.
	ErrNoBackendAddress = errors.New("Backend has no reachable IPv4 address")
	// ErrLoopbackBackend means a backend address is loopback or in 0.0.0.0/8.
	//nolint:staticcheck // Preserve the Rust user-facing error verbatim.
	ErrLoopbackBackend = errors.New("Backend URL must be reachable from Cloud, not host loopback")
)

// OverlapError identifies a host route overlapping the configured pool.
type OverlapError struct {
	// Route is the conflicting IPv4 route.
	Route netip.Prefix
}

// Error describes the conflicting route.
func (e *OverlapError) Error() string { panic("not written: m-network") }

// FirewallError reports a refused Cloud firewall transaction.
type FirewallError struct {
	// Source is the underlying nftables failure.
	Source error
}

// Error describes the failed firewall transaction.
func (e *FirewallError) Error() string { panic("not written: m-network") }

// Unwrap preserves the underlying failure for errors.Is and errors.As.
func (e *FirewallError) Unwrap() error { panic("not written: m-network") }
