//go:build linux

package network_test

import (
	"context"
	"net/netip"
	"net/url"
	"os"
	"os/exec"
	"strings"
	"testing"

	"github.com/wspl/demi/go/machines/internal/network"
	"github.com/wspl/demi/go/machines/internal/roottest"
	"github.com/wspl/demi/go/machines/internal/tools"
)

func TestMain(m *testing.M) {
	os.Exit(roottest.Main(m))
}

// run runs a program of the test's own namespaces and returns its output.
func run(t *testing.T, program string, args ...string) string {
	t.Helper()
	output, err := exec.Command(program, args...).CombinedOutput()
	if err != nil {
		t.Fatalf("%s %v: %v\n%s", program, args, err, output)
	}
	return string(output)
}

func read(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

func newNetwork(t *testing.T, pool, backend string) *network.Network {
	t.Helper()
	programs, err := tools.Resolve(os.Args[0])
	if err != nil {
		t.Skip(err)
	}
	address, err := url.Parse(backend)
	if err != nil {
		t.Fatal(err)
	}
	return network.New(netip.MustParsePrefix(pool), []netip.Addr{netip.MustParseAddr("1.1.1.1"), netip.MustParseAddr("8.8.8.8")}, address, programs)
}

// The table the manager installs lists as the recorded fixture: nft compiles the
// JSON into the same rules the Rust manager's transaction did.
func TestASlotAttachesAndDetachesUnderTheInstalledPolicy(t *testing.T) {
	roottest.Require(t)
	ctx := context.Background()
	net := newNetwork(t, "172.30.0.0/16", "http://203.0.113.10:3271")
	if err := net.Prepare(ctx); err != nil {
		t.Fatal(err)
	}
	if got := strings.TrimSpace(read(t, "/proc/sys/net/ipv4/ip_forward")); got != "1" {
		t.Errorf("ip_forward %s", got)
	}
	fixture := read(t, "../../../../crates/machines/tests/fixtures/nft-ruleset.txt")
	if listing := run(t, "nft", "list", "table", "inet", "demi_cloud"); listing != fixture {
		t.Errorf("the installed table:\n%s\nwant:\n%s", listing, fixture)
	}
	// Installing it again replaces it.
	if err := net.Prepare(ctx); err != nil {
		t.Fatal(err)
	}
	if listing := run(t, "nft", "list", "table", "inet", "demi_cloud"); listing != fixture {
		t.Errorf("the table installed again:\n%s", listing)
	}

	slot := network.NewSlot(netip.MustParsePrefix("172.30.0.0/16"), 3)
	if err := net.Attach(ctx, slot); err != nil {
		t.Fatal(err)
	}
	if set := run(t, "nft", "list", "set", "inet", "demi_cloud", "slots"); !strings.Contains(set, `"demih3" . 172.30.0.14`) {
		t.Errorf("the slots set:\n%s", set)
	}
	if out := run(t, "ip", "-o", "addr", "show", "demih3"); !strings.Contains(out, "172.30.0.13/30") {
		t.Errorf("the host end:\n%s", out)
	}
	if out := run(t, "ip", "-n", "demi-3", "-o", "addr", "show", "demip3"); !strings.Contains(out, "172.30.0.14/30") {
		t.Errorf("the sandbox end:\n%s", out)
	}
	if out := run(t, "ip", "-n", "demi-3", "route"); !strings.Contains(out, "default via 172.30.0.13") {
		t.Errorf("the sandbox's routes:\n%s", out)
	}
	// IPv6 is off on both ends, unless the kernel runs without it.
	if _, err := os.Stat("/proc/sys/net/ipv6"); err == nil {
		if got := strings.TrimSpace(read(t, "/proc/sys/net/ipv6/conf/demih3/disable_ipv6")); got != "1" {
			t.Errorf("the host end has IPv6 on: %s", got)
		}
		out := run(t, "ip", "netns", "exec", "demi-3", "cat", "/proc/sys/net/ipv6/conf/all/disable_ipv6")
		if strings.TrimSpace(out) != "1" {
			t.Errorf("the sandbox has IPv6 on: %s", out)
		}
	}

	if err := net.Detach(ctx, slot); err != nil {
		t.Fatal(err)
	}
	if set := run(t, "nft", "list", "set", "inet", "demi_cloud", "slots"); strings.Contains(set, "demih3") {
		t.Errorf("the slots set after the detach:\n%s", set)
	}
	for _, path := range []string{"/sys/class/net/demih3", "/run/netns/demi-3"} {
		if _, err := os.Stat(path); err == nil {
			t.Errorf("%s remains", path)
		}
	}
	// Detaching what is gone is fine, with or without the table.
	if err := net.Detach(ctx, slot); err != nil {
		t.Fatal(err)
	}
	run(t, "nft", "delete", "table", "inet", "demi_cloud")
	if err := net.Detach(ctx, slot); err != nil {
		t.Fatal(err)
	}
}

func TestAPoolThatOverlapsAHostRouteOrALoopbackBackendIsRefused(t *testing.T) {
	roottest.Require(t)
	ctx := context.Background()
	run(t, "ip", "link", "set", "lo", "up")
	// A veth pair, which the manager needs anyway: a kernel may lack the dummy
	// driver.
	run(t, "ip", "link", "add", "probe0", "type", "veth", "peer", "name", "probe1")
	run(t, "ip", "addr", "add", "172.30.5.1/24", "dev", "probe0")
	run(t, "ip", "link", "set", "probe0", "up")
	overlapping := newNetwork(t, "172.30.0.0/16", "http://203.0.113.10:3271")
	err := overlapping.Prepare(ctx)
	if err == nil || err.Error() != "DEMI_MANAGED_SUBNET overlaps host route 172.30.5.0/24" {
		t.Errorf("an overlapping pool: %v", err)
	}
	for _, backend := range []string{"http://127.0.0.1:3271", "http://0.1.2.3:3271"} {
		err := newNetwork(t, "10.99.0.0/16", backend).Prepare(ctx)
		if err == nil || err.Error() != "Backend URL must be reachable from Cloud, not host loopback" {
			t.Errorf("%s: %v", backend, err)
		}
	}
	err = newNetwork(t, "10.99.0.0/16", "http://[::1]:3271").Prepare(ctx)
	if err == nil || err.Error() != "Backend has no reachable IPv4 address" {
		t.Errorf("an IPv6 backend: %v", err)
	}
}
