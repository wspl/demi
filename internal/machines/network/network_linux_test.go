//go:build linux

package network_test

import (
	"context"
	_ "embed"
	"errors"
	"fmt"
	"net/netip"
	"net/url"
	"os"
	"os/exec"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machines/network"
	"github.com/wspl/demi/internal/machines/system/systemtest"
	"go.uber.org/goleak"
)

//go:embed testdata/nft-ruleset.txt
var expectedRules string

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// TestSlotAttachesAndDetachesUnderInstalledPolicy is the root attach and detach scenario.
// It uses real kernel namespaces and readback tools; it normally costs <1 s.
func TestSlotAttachesAndDetachesUnderInstalledPolicy(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("needs explicit root invocation")
	}
	err := systemtest.Isolate(t.Context(), func(ctx context.Context) (result error) {
		nft, err := systemtest.NftPath(ctx)
		if err != nil {
			return err
		}
		backend, err := url.Parse("http://203.0.113.10:3271")
		if err != nil {
			return err
		}
		pool := netip.MustParsePrefix("172.30.0.0/16")
		n := network.New(pool, []netip.Addr{netip.MustParseAddr("1.1.1.1"), netip.MustParseAddr("8.8.8.8")}, *backend)
		for range 2 {
			if err := n.Prepare(ctx); err != nil {
				return err
			}
			enabled, err := os.ReadFile("/proc/sys/net/ipv4/ip_forward")
			if err != nil {
				return err
			}
			if strings.TrimSpace(string(enabled)) != "1" {
				return fmt.Errorf("forwarding: %q", enabled)
			}
			listing, err := commandOutput(ctx, nft, "list", "table", "inet", "demi_cloud")
			if err != nil {
				return err
			}
			if listing != expectedRules {
				return fmt.Errorf("installed table differs:\n%s\nwant:\n%s", listing, expectedRules)
			}
		}
		slot := network.NewPool(pool, 8).Slot(3)
		// Register before Attach: failure can leave a partially attached slot.
		defer func() { result = errors.Join(result, n.Detach(context.WithoutCancel(ctx), slot)) }()
		if err := n.Attach(ctx, slot); err != nil {
			return err
		}
		checks := []struct {
			program  string
			args     []string
			contains string
		}{
			{nft, []string{"list", "set", "inet", "demi_cloud", "slots"}, `"demih3" . 172.30.0.14`},
			{"ip", []string{"-o", "addr", "show", "demih3"}, "172.30.0.13/30"},
			{"ip", []string{"-n", "demi-3", "-o", "addr", "show", "demip3"}, "172.30.0.14/30"},
			{"ip", []string{"-n", "demi-3", "route"}, "default via 172.30.0.13"},
		}
		for _, check := range checks {
			output, err := commandOutput(ctx, check.program, check.args...)
			if err != nil {
				return err
			}
			if !strings.Contains(output, check.contains) {
				return fmt.Errorf("%s %v: missing %q in %s", check.program, check.args, check.contains, output)
			}
		}
		if _, err := os.Stat("/proc/sys/net/ipv6"); err == nil {
			disabled, err := os.ReadFile("/proc/sys/net/ipv6/conf/demih3/disable_ipv6")
			if err != nil {
				return err
			}
			if strings.TrimSpace(string(disabled)) != "1" {
				return fmt.Errorf("host IPv6 enabled: %q", disabled)
			}
			disabledInside, err := commandOutput(
				ctx,
				"ip",
				"netns",
				"exec",
				"demi-3",
				"cat",
				"/proc/sys/net/ipv6/conf/all/disable_ipv6",
			)
			if err != nil {
				return err
			}
			if strings.TrimSpace(disabledInside) != "1" {
				return fmt.Errorf("peer IPv6 enabled: %q", disabledInside)
			}
		} else if !errors.Is(err, os.ErrNotExist) {
			return err
		}
		if err := n.Detach(ctx, slot); err != nil {
			return err
		}
		listing, err := commandOutput(ctx, nft, "list", "set", "inet", "demi_cloud", "slots")
		if err != nil {
			return err
		}
		if strings.Contains(listing, "demih3") {
			return fmt.Errorf("slot still admitted: %s", listing)
		}
		links, err := commandOutput(ctx, "ip", "-o", "link", "show")
		if err != nil {
			return err
		}
		if strings.Contains(links, "demih3") {
			return fmt.Errorf("host link remains: %s", links)
		}
		if _, err := os.Stat("/run/netns/demi-3"); !errors.Is(err, os.ErrNotExist) {
			return fmt.Errorf("namespace remains: %v", err)
		}
		if err := n.Detach(ctx, slot); err != nil {
			return err
		}
		if _, err := commandOutput(ctx, nft, "delete", "table", "inet", "demi_cloud"); err != nil {
			return err
		}
		return n.Detach(ctx, slot)
	})
	if err != nil {
		t.Fatal(err)
	}
}

// TestOverlappingRouteAndLoopbackBackendRefused is the root refusal scenario.
// Kernel route setup requires an isolated root namespace (<1 s).
func TestOverlappingRouteAndLoopbackBackendRefused(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("needs explicit root invocation")
	}
	err := systemtest.Isolate(t.Context(), func(ctx context.Context) error {
		for _, args := range [][]string{
			{"link", "set", "lo", "up"},
			{"link", "add", "probe0", "type", "veth", "peer", "name", "probe1"},
			{"addr", "add", "172.30.5.1/24", "dev", "probe0"},
			{"link", "set", "probe0", "up"},
		} {
			if _, err := commandOutput(ctx, "ip", args...); err != nil {
				return err
			}
		}
		dns := []netip.Addr{netip.MustParseAddr("1.1.1.1")}
		backend, err := url.Parse("http://203.0.113.10:3271")
		if err != nil {
			return err
		}
		n := network.New(netip.MustParsePrefix("172.30.0.0/16"), dns, *backend)
		err = n.Prepare(ctx)
		if err == nil || err.Error() != "DEMI_MANAGED_SUBNET overlaps host route 172.30.5.0/24" {
			return fmt.Errorf("overlap refusal: %v", err)
		}
		for _, text := range []string{"http://127.0.0.1:3271", "http://0.1.2.3:3271"} {
			backend, err := url.Parse(text)
			if err != nil {
				return err
			}
			n := network.New(netip.MustParsePrefix("10.99.0.0/16"), dns, *backend)
			if err := n.Prepare(
				ctx,
			); !errors.Is(err, network.ErrLoopbackBackend) ||
				err.Error() != "Backend URL must be reachable from Cloud, not host loopback" {
				return fmt.Errorf("%s refusal: %v", text, err)
			}
		}
		backend, err = url.Parse("http://[2001:db8::1]:3271")
		if err != nil {
			return err
		}
		noIPv4 := network.New(netip.MustParsePrefix("10.99.0.0/16"), dns, *backend)
		if err := noIPv4.Prepare(ctx); !errors.Is(err, network.ErrNoBackendAddress) {
			return fmt.Errorf("IPv6-only backend refusal: %v", err)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

// commandOutput runs a kernel inspection or setup tool on the isolated test
// thread. CommandContext owns cancellation and CombinedOutput waits and reaps.
func commandOutput(ctx context.Context, program string, args ...string) (string, error) {
	command := exec.CommandContext(ctx, program, args...)
	command.Env = append(os.Environ(), "LC_ALL=C.UTF-8")
	output, err := command.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("%s %v: %s: %w", program, args, output, err)
	}
	return string(output), nil
}

// A failed firewall admission leaves kernel resources for the sandbox owner's
// cleanup. This real-kernel scenario verifies cleanup and reuse (<1 s).
func TestFailedAttachCanBeCleanedAndReused(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("needs explicit root invocation")
	}
	err := systemtest.Isolate(t.Context(), func(ctx context.Context) (result error) {
		backend, err := url.Parse("http://203.0.113.10:3271")
		if err != nil {
			return err
		}
		pool := network.NewPool(netip.MustParsePrefix("172.30.0.0/16"), 1)
		lease, err := pool.Take()
		if err != nil {
			return err
		}
		defer lease.Release()
		slot := lease.Slot()
		n := network.New(netip.MustParsePrefix("172.30.0.0/16"), []netip.Addr{netip.MustParseAddr("1.1.1.1")}, *backend)
		defer func() { result = errors.Join(result, n.Detach(context.WithoutCancel(ctx), slot)) }()
		// No policy exists yet: the final admission step must fail.
		err = n.Attach(ctx, slot)
		if err == nil || !strings.HasPrefix(err.Error(), "Cannot apply the Cloud firewall: ") ||
			errors.Unwrap(err) == nil {
			return fmt.Errorf("missing firewall refusal: %v", err)
		}
		if err := n.Detach(ctx, slot); err != nil {
			return err
		}
		if err := n.Prepare(ctx); err != nil {
			return err
		}
		if err := n.Attach(ctx, slot); err != nil {
			return fmt.Errorf("reuse after partial attachment: %w", err)
		}
		canceled, cancel := context.WithCancel(ctx)
		cancel()
		if err := n.Detach(canceled, slot); !errors.Is(err, context.Canceled) {
			return fmt.Errorf("canceled detach: %v", err)
		}
		if _, err := os.Stat("/run/netns/demi-0"); err != nil {
			return fmt.Errorf("canceled detach changed namespace: %w", err)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
