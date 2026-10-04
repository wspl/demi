//go:build linux

package network_test

import (
	"errors"
	"net/netip"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/network"
)

func TestSlotsDisjointAndReusedAfterRelease(t *testing.T) {
	pool := network.NewPool(netip.MustParsePrefix("172.30.0.0/16"), 3)
	a, err := pool.Take()
	if err != nil {
		t.Fatal(err)
	}
	defer a.Release()
	b, err := pool.Take()
	if err != nil {
		t.Fatal(err)
	}
	defer b.Release()
	for _, check := range []struct {
		slot             network.Slot
		index            uint16
		gateway, address string
	}{
		{a.Slot(), 0, "172.30.0.1", "172.30.0.2"},
		{b.Slot(), 1, "172.30.0.5", "172.30.0.6"},
		{pool.Slot(64), 64, "172.30.1.1", "172.30.1.2"},
	} {
		if check.slot.Index != check.index || check.slot.Gateway != netip.MustParseAddr(check.gateway) ||
			check.slot.Address != netip.MustParseAddr(check.address) {
			t.Errorf("slot: %+v; want %d %s %s", check.slot, check.index, check.gateway, check.address)
		}
	}
	far := pool.Slot(64)
	if far.HostInterface() != "demih64" || far.PeerInterface() != "demip64" || far.Namespace() != "demi-64" {
		t.Errorf("slot names: %s %s %s", far.HostInterface(), far.PeerInterface(), far.Namespace())
	}
	c, err := pool.Take()
	if err != nil {
		t.Fatal(err)
	}
	defer c.Release()
	if c.Slot().Index != 2 {
		t.Fatalf("third slot: %+v", c.Slot())
	}
	if _, err := pool.Take(); !errors.Is(err, network.ErrExhausted) ||
		err.Error() != "All Cloud network slots are in use" {
		t.Fatalf("exhaustion: %v", err)
	}
	second := b.Slot()
	b.Release()
	reused, err := pool.Take()
	if err != nil {
		t.Fatal(err)
	}
	defer reused.Release()
	if reused.Slot() != second {
		t.Errorf("reused %+v; want %+v", reused.Slot(), second)
	}
	// Explicit release is idempotent even after another owner reuses the slot.
	b.Release()
	if _, err := pool.Take(); !errors.Is(err, network.ErrExhausted) {
		t.Fatalf("old lease freed new owner: %v", err)
	}
	a.Release()
	c.Release()
	reused.Release()
	if !pool.Contains(2) || pool.Contains(3) {
		t.Error("pool boundary")
	}
}

// Concurrent manager workers must never hold the same slot simultaneously.
func TestConcurrentSlotOwners(t *testing.T) {
	const count = 32
	pool := network.NewPool(netip.MustParsePrefix("172.30.0.0/16"), count)
	leases := make(chan *network.Lease, count)
	var workers sync.WaitGroup
	for range count {
		workers.Go(func() {
			lease, err := pool.Take()
			if err != nil {
				t.Error(err)
				return
			}
			leases <- lease
		})
	}
	workers.Wait()
	close(leases)
	seen := make(map[uint16]bool)
	for lease := range leases {
		if seen[lease.Slot().Index] {
			t.Errorf("duplicate slot %d", lease.Slot().Index)
		}
		seen[lease.Slot().Index] = true
		workers.Go(lease.Release)
		workers.Go(lease.Release)
	}
	workers.Wait()
	if len(seen) != count {
		t.Fatalf("owners: %d", len(seen))
	}
	var reclaimed []*network.Lease
	defer func() {
		for _, lease := range reclaimed {
			lease.Release()
		}
	}()
	for range count {
		lease, err := pool.Take()
		if err != nil {
			t.Fatal(err)
		}
		reclaimed = append(reclaimed, lease)
	}
}
