package network_test

import (
	"encoding/json/v2"
	"net/netip"
	"reflect"
	"testing"

	"github.com/wspl/demi/go/machines/internal/network"
)

func TestSlotsHaveDisjointNetworksAndAreReusedAfterRelease(t *testing.T) {
	pool := network.NewSlotPool(netip.MustParsePrefix("172.30.0.0/16"), 3)
	a, err := pool.Take()
	if err != nil {
		t.Fatal(err)
	}
	b, err := pool.Take()
	if err != nil {
		t.Fatal(err)
	}
	first, second := a.Slot(), b.Slot()
	if first.Index != 0 || first.Gateway.String() != "172.30.0.1" || first.Address.String() != "172.30.0.2" {
		t.Errorf("slot 0: %+v", first)
	}
	if second.Index != 1 || second.Gateway.String() != "172.30.0.5" || second.Address.String() != "172.30.0.6" {
		t.Errorf("slot 1: %+v", second)
	}
	far := pool.Slot(64)
	if far.Gateway.String() != "172.30.1.1" || far.Address.String() != "172.30.1.2" {
		t.Errorf("slot 64: %+v", far)
	}
	if far.HostInterface() != "demih64" || far.PeerInterface() != "demip64" || far.Namespace() != "demi-64" {
		t.Errorf("names %s %s %s", far.HostInterface(), far.PeerInterface(), far.Namespace())
	}
	c, err := pool.Take()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := pool.Take(); err == nil || err.Error() != "All Cloud network slots are in use" {
		t.Errorf("an exhausted pool: %v", err)
	}
	b.Release()
	// Releasing twice is the same as releasing once.
	b.Release()
	reused, err := pool.Take()
	if err != nil || reused.Slot() != second {
		t.Fatalf("slot 1 again: %+v, %v", reused, err)
	}
	a.Release()
	c.Release()
	reused.Release()
	if !pool.Contains(2) || pool.Contains(3) {
		t.Error("the pool holds slots 0 to 2")
	}
}

func TestTheSlotsSetChangesOnePairAtATime(t *testing.T) {
	slot := network.NewSlot(netip.MustParsePrefix("172.30.0.0/16"), 3)
	got, err := network.AddSlotTransaction(slot).Encode()
	if err != nil {
		t.Fatal(err)
	}
	const element = `{"family":"inet","table":"demi_cloud","name":"slots","elem":[{"concat":["demih3","172.30.0.14"]}]}`
	if want := `{"nftables":[{"add":{"element":` + element + `}}]}`; string(got) != want {
		t.Errorf("adding a slot:\n got %s\nwant %s", got, want)
	}
	// A removal adds the table, the set and the element first, so it succeeds
	// whether or not they exist.
	got, err = network.RemoveSlotTransaction(slot).Encode()
	if err != nil {
		t.Fatal(err)
	}
	want := `{"nftables":[` +
		`{"add":{"table":{"family":"inet","name":"demi_cloud"}}},` +
		`{"add":{"set":{"family":"inet","table":"demi_cloud","name":"slots","type":["ifname","ipv4_addr"]}}},` +
		`{"add":{"element":` + element + `}},` +
		`{"delete":{"element":` + element + `}}]}`
	if string(got) != want {
		t.Errorf("removing a slot:\n got %s\nwant %s", got, want)
	}
}

func TestTheTableReplacesItselfInOneTransaction(t *testing.T) {
	transaction := network.TableTransaction(network.Policy{
		Pool:        netip.MustParsePrefix("172.30.0.0/16"),
		Backend:     []netip.Addr{netip.MustParseAddr("203.0.113.10")},
		BackendPort: 3271,
		DNS:         []netip.Addr{netip.MustParseAddr("1.1.1.1"), netip.MustParseAddr("8.8.8.8")},
	})
	data, err := transaction.Encode()
	if err != nil {
		t.Fatal(err)
	}
	var decoded struct {
		Nftables []map[string]map[string]map[string]any `json:"nftables"`
	}
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatal(err)
	}
	// Adding the table first makes its deletion succeed whether or not it
	// existed; then it is made again, with its set and chains, then the rules.
	var steps []string
	for _, command := range decoded.Nftables {
		for verb, object := range command {
			for kind := range object {
				steps = append(steps, verb+" "+kind)
			}
		}
	}
	want := []string{"add table", "delete table", "add table", "add set",
		"add chain", "add chain", "add chain", "add chain",
		"add rule", "add rule", "add rule", "add rule", "add rule", "add rule", "add rule", "add rule", "add rule", "add rule", "add rule", "add rule"}
	if !reflect.DeepEqual(steps, want) {
		t.Errorf("steps %v", steps)
	}
}
