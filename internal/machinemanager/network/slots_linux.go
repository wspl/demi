//go:build linux

package network

import (
	"encoding/binary"
	"fmt"
	"net/netip"
	"sync"
)

const hostInterfacePrefix = "demih"

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
func SlotAt(subnet netip.Prefix, index uint16) Slot {
	first := subnet.Masked().Addr().As4()
	base := binary.BigEndian.Uint32(first[:]) + uint32(index)*4
	var gateway, address [4]byte
	binary.BigEndian.PutUint32(gateway[:], base+1)
	binary.BigEndian.PutUint32(address[:], base+2)
	return Slot{Index: index, Gateway: netip.AddrFrom4(gateway), Address: netip.AddrFrom4(address)}
}

// HostInterface returns the host end of the sandbox's veth pair, such as demih3.
func (s Slot) HostInterface() string {
	return fmt.Sprintf("%s%d", hostInterfacePrefix, s.Index)
}

// PeerInterface returns the sandbox's end of the veth pair, such as demip3.
func (s Slot) PeerInterface() string {
	return fmt.Sprintf("demip%d", s.Index)
}

// Namespace returns the sandbox's network namespace, such as demi-3.
func (s Slot) Namespace() string {
	return fmt.Sprintf("demi-%d", s.Index)
}

// Pool owns the slots of the configured pool. Its methods are safe for
// concurrent use. Construct it with NewPool and do not copy it.
type Pool struct {
	subnet netip.Prefix
	// mu protects taken; allocation never waits while holding it.
	mu    sync.Mutex
	taken []bool
}

// NewPool creates an empty slot pool. The configuration must supply an IPv4
// subnet large enough for count disjoint /30 networks.
func NewPool(subnet netip.Prefix, count uint16) *Pool {
	return &Pool{subnet: subnet, taken: make([]bool, int(count))}
}

// Slot returns the slot at index, taken or not, as a recovery record names it.
func (p *Pool) Slot(index uint16) Slot {
	return SlotAt(p.subnet, index)
}

// Contains reports whether index lies in the configured pool.
func (p *Pool) Contains(index uint16) bool {
	return int(index) < len(p.taken)
}

// Take takes the lowest free slot, or returns ErrExhausted. The caller owns
// the lease and must defer Release or transfer that duty to the sandbox owner.
func (p *Pool) Take() (*Lease, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for index, taken := range p.taken {
		if !taken {
			p.taken[index] = true
			return &Lease{slot: p.Slot(uint16(index)), pool: p}, nil
		}
	}
	return nil, ErrExhausted
}

// Lease holds a slot for a running sandbox. Do not copy it. Releasing the
// allocation does not detach its networking; the owner must detach it first.
type Lease struct {
	slot Slot
	pool *Pool
	once sync.Once
}

// Slot returns the leased slot as a value, which remains readable after release.
func (l *Lease) Slot() Slot {
	return l.slot
}

// Release frees the allocation for reuse. It is idempotent and safe to call
// concurrently; it does not wait for or perform kernel cleanup.
func (l *Lease) Release() {
	l.once.Do(func() {
		l.pool.mu.Lock()
		defer l.pool.mu.Unlock()
		l.pool.taken[l.slot.Index] = false
	})
}
