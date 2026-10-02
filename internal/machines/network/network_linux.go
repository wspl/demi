//go:build linux

package network

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"net/netip"
	"net/url"
)

// Network is the host side of Cloud networking. Configuration is immutable;
// the manager serializes Prepare with slot operations and operations on the
// same slot. Construct it with New.
type Network struct{}

// New creates the host network policy from the validated IPv4 pool, IPv4 DNS
// resolvers and HTTP(S) backend URL. It copies the supplied configuration.
// Firewall transactions use nftables directly with an explicit namespace FD.
func New(pool netip.Prefix, dns []netip.Addr, backend url.URL) *Network {
	panic("not written: m-network")
}

// Prepare checks that the pool overlaps no host route, resolves the backend,
// enables forwarding, and replaces the firewall table. Recovery calls this
// after detaching recorded slots; replacement starts with an empty slot set.
func (n *Network) Prepare(ctx context.Context) error { panic("not written: m-network") }

// Attach creates slot's namespace and veth pair, gives both ends their
// addresses with IPv6 off, routes the sandbox through the host end, and admits
// the slot's pair to the firewall. A kernel without IPv6 has none to turn off.
// On failure the owner must call Detach with a cleanup context to remove any
// partially created resources before releasing the slot lease.
func (n *Network) Attach(ctx context.Context, slot Slot) error { panic("not written: m-network") }

// Detach removes slot's firewall pair, veth pair and namespace, each only if
// it exists. Recovery may pass a slot reconstructed from a saved record.
// Cleanup callers use a context without cancellation and await completion.
func (n *Network) Detach(ctx context.Context, slot Slot) error { panic("not written: m-network") }
