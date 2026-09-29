// Package network is the host side of sandbox networking
// (docs/cloud/managed-hosts.md § Networking): a network namespace and veth pair
// per running sandbox, configured through netlink, and one firewall table for
// all of them.
package network

import (
	"errors"
	"fmt"
	"net/netip"
	"sync"
)

// A Slot is one sandbox's network: the /30 at Index in the pool.
type Slot struct {
	Index uint16
	// Gateway is the host's address, the sandbox's gateway.
	Gateway netip.Addr
	// Address is the sandbox's address.
	Address netip.Addr
}

// NewSlot returns the slot at index of subnet, which the configuration made large
// enough for every index below its slot count.
func NewSlot(subnet netip.Prefix, index uint16) Slot {
	first := subnet.Masked().Addr().As4()
	base := uint32(first[0])<<24 | uint32(first[1])<<16 | uint32(first[2])<<8 | uint32(first[3])
	base += uint32(index) * 4
	address := func(offset uint32) netip.Addr {
		value := base + offset
		return netip.AddrFrom4([4]byte{byte(value >> 24), byte(value >> 16), byte(value >> 8), byte(value)})
	}
	return Slot{Index: index, Gateway: address(1), Address: address(2)}
}

// HostInterface returns the host end of the sandbox's veth pair, such as
// demih3.
func (s Slot) HostInterface() string { return fmt.Sprintf("demih%d", s.Index) }

// PeerInterface returns the sandbox's end of the veth pair, such as demip3.
func (s Slot) PeerInterface() string { return fmt.Sprintf("demip%d", s.Index) }

// Namespace returns the sandbox's network namespace, such as demi-3.
func (s Slot) Namespace() string { return fmt.Sprintf("demi-%d", s.Index) }

// ErrExhausted means every slot is in use.
var ErrExhausted = errors.New("All Cloud network slots are in use")

// A SlotPool hands out the slots of the configured pool, each to one sandbox at
// a time: the lowest free /30 (docs/cloud/setup.md § Configuration).
type SlotPool struct {
	subnet netip.Prefix
	count  uint16

	mu    sync.Mutex
	taken map[uint16]bool
}

// NewSlotPool returns a pool of count slots of subnet.
func NewSlotPool(subnet netip.Prefix, count uint16) *SlotPool {
	return &SlotPool{subnet: subnet, count: count, taken: make(map[uint16]bool)}
}

// Slot returns the slot at index, taken or not, as a record names it.
func (p *SlotPool) Slot(index uint16) Slot { return NewSlot(p.subnet, index) }

// Contains reports whether index lies in the configured pool.
func (p *SlotPool) Contains(index uint16) bool { return index < p.count }

// Take takes the lowest free slot.
func (p *SlotPool) Take() (*SlotLease, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for index := uint16(0); index < p.count; index++ {
		if !p.taken[index] {
			p.taken[index] = true
			return &SlotLease{slot: p.Slot(index), pool: p}, nil
		}
	}
	return nil, ErrExhausted
}

// A SlotLease is a slot a running sandbox holds; releasing it frees the slot.
type SlotLease struct {
	slot Slot
	pool *SlotPool
	once sync.Once
}

// Slot returns the leased slot.
func (l *SlotLease) Slot() Slot { return l.slot }

// Release frees the slot; releasing twice is the same as releasing once.
func (l *SlotLease) Release() {
	l.once.Do(func() {
		l.pool.mu.Lock()
		defer l.pool.mu.Unlock()
		delete(l.pool.taken, l.slot.Index)
	})
}
