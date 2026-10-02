//go:build linux

package network

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "net/netip"

// Slot is one sandbox's network: the /30 at Index in the pool.
type Slot struct {
	// Index identifies the /30 in the configured pool.
	Index uint16
	// Gateway is the host's address, the sandbox's gateway.
	Gateway netip.Addr
	// Address is the sandbox's address.
	Address netip.Addr
}

// SlotAt returns the slot at index of subnet, which the configuration made
// large enough for every index below its slot count. Subnet must be IPv4.
func SlotAt(subnet netip.Prefix, index uint16) Slot { panic("not written: m-network") }

// HostInterface returns the host end of the sandbox's veth pair, such as demih3.
func (s Slot) HostInterface() string { panic("not written: m-network") }

// PeerInterface returns the sandbox's end of the veth pair, such as demip3.
func (s Slot) PeerInterface() string { panic("not written: m-network") }

// Namespace returns the sandbox's network namespace, such as demi-3.
func (s Slot) Namespace() string { panic("not written: m-network") }

// Pool owns the slots of the configured pool. Its methods are safe for
// concurrent use. Construct it with NewPool and do not copy it.
type Pool struct{}

// NewPool creates an empty slot pool. The configuration must supply an IPv4
// subnet large enough for count disjoint /30 networks.
func NewPool(subnet netip.Prefix, count uint16) *Pool { panic("not written: m-network") }

// Slot returns the slot at index, taken or not, as a recovery record names it.
func (p *Pool) Slot(index uint16) Slot { panic("not written: m-network") }

// Contains reports whether index lies in the configured pool.
func (p *Pool) Contains(index uint16) bool { panic("not written: m-network") }

// Take takes the lowest free slot, or returns ErrExhausted. The caller owns
// the lease and must defer Release or transfer that duty to the sandbox owner.
func (p *Pool) Take() (*Lease, error) { panic("not written: m-network") }

// Lease holds a slot for a running sandbox. Do not copy it. Releasing the
// allocation does not detach its networking; the owner must detach it first.
type Lease struct{}

// Slot returns the leased slot as a value, which remains readable after release.
func (l *Lease) Slot() Slot { panic("not written: m-network") }

// Release frees the allocation for reuse. It is idempotent and safe to call
// concurrently; it does not wait for or perform kernel cleanup.
func (l *Lease) Release() { panic("not written: m-network") }
