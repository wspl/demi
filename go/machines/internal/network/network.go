//go:build linux

package network

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"os"
	"os/exec"
	"slices"
	"strconv"
	"strings"

	whatwg "github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/tools"
)

// hostInterfaces is the prefix of every host interface of a Cloud slot.
const hostInterfaces = "demih"

// ipv6Settings is the kernel's IPv6 settings, absent when it runs without IPv6,
// such as when booted with ipv6.disable=1.
const ipv6Settings = "/proc/sys/net/ipv6"

// Errors of the host side of networking.
var (
	// ErrNoBackendAddress means the backend has no IPv4 address to reach.
	ErrNoBackendAddress = errors.New("Backend has no reachable IPv4 address")
	// ErrLoopbackBackend means a sandbox cannot reach the backend, which is on
	// the host's loopback.
	ErrLoopbackBackend = errors.New("Backend URL must be reachable from Cloud, not host loopback")
)

// A FirewallError means nft refused the firewall's transaction; it carries what
// nft printed.
type FirewallError struct {
	Message string
}

func (e *FirewallError) Error() string { return "Cannot apply the Cloud firewall: " + e.Message }

// An OverlapError means the pool overlaps a route of the host.
type OverlapError struct {
	Route netip.Prefix
}

func (e *OverlapError) Error() string {
	return "DEMI_MANAGED_SUBNET overlaps host route " + e.Route.String()
}

// Network is the host side of Cloud networking.
type Network struct {
	pool    netip.Prefix
	dns     []netip.Addr
	backend *whatwg.Url
	tools   *tools.Tools
}

// New returns the host side of Cloud networking for a pool, the resolvers a
// sandbox uses and the backend it may reach, whose URL is in its normal form.
func New(pool netip.Prefix, dns []netip.Addr, backend *whatwg.Url, t *tools.Tools) *Network {
	return &Network{pool: pool, dns: dns, backend: backend, tools: t}
}

// apply applies transaction as one nft transaction; a refusal carries what nft
// printed.
func (n *Network) apply(ctx context.Context, transaction Transaction) error {
	data, err := transaction.Encode()
	if err != nil {
		return err
	}
	command := n.tools.Command(ctx, tools.Nft, "-j", "-f", "-")
	command.Stdin = bytes.NewReader(data)
	var stderr bytes.Buffer
	command.Stderr = &stderr
	if err := command.Run(); err != nil {
		var exit *exec.ExitError
		if errors.As(err, &exit) {
			return &FirewallError{Message: strings.TrimSpace(stderr.String())}
		}
		return &FirewallError{Message: err.Error()}
	}
	return nil
}

// Prepare checks that the pool overlaps no host route, resolves the backend,
// enables forwarding, and installs the firewall table.
func (n *Network) Prepare(ctx context.Context) error {
	routes, err := mainRoutes()
	if err != nil {
		return err
	}
	for _, route := range routes {
		ours := strings.HasPrefix(route.interface_, hostInterfaces)
		if route.network.Bits() == 0 || ours {
			continue
		}
		if n.pool.Contains(route.network.Addr()) || route.network.Contains(n.pool.Masked().Addr()) {
			return &OverlapError{Route: route.network}
		}
	}
	backend, err := n.backendAddresses(ctx)
	if err != nil {
		return err
	}
	port, err := backendPort(n.backend)
	if err != nil {
		return err
	}
	table := TableTransaction(Policy{Pool: n.pool, Backend: backend, BackendPort: port, DNS: n.dns})
	if err := os.WriteFile("/proc/sys/net/ipv4/ip_forward", []byte("1"), 0o644); err != nil {
		return err
	}
	return n.apply(ctx, table)
}

// backendPort returns the port of the backend's URL, or its scheme's default.
func backendPort(backend *whatwg.Url) (uint16, error) {
	text := backend.Port()
	if text == "" {
		switch backend.Scheme() {
		case "https", "wss":
			return 443, nil
		default:
			return 80, nil
		}
	}
	port, err := strconv.ParseUint(text, 10, 16)
	if err != nil {
		return 0, fmt.Errorf("the backend URL has no valid port: %w", err)
	}
	return uint16(port), nil
}

// backendAddresses returns the backend's IPv4 addresses, none of them loopback
// or unspecified: a sandbox reaches the backend over the network.
func (n *Network) backendAddresses(ctx context.Context) ([]netip.Addr, error) {
	// The standard writes an IPv6 host in brackets.
	host := strings.TrimSuffix(strings.TrimPrefix(n.backend.Hostname(), "["), "]")
	var addresses []netip.Addr
	if literal, err := netip.ParseAddr(host); err == nil {
		if literal.Is4() {
			addresses = []netip.Addr{literal}
		}
	} else if host != "" {
		found, err := net.DefaultResolver.LookupNetIP(ctx, "ip4", host)
		if err != nil {
			return nil, err
		}
		for _, address := range found {
			if address.Is4() {
				addresses = append(addresses, address)
			}
		}
	}
	slices.SortFunc(addresses, netip.Addr.Compare)
	addresses = slices.Compact(addresses)
	if len(addresses) == 0 {
		return nil, ErrNoBackendAddress
	}
	for _, address := range addresses {
		if address.IsLoopback() || address.As4()[0] == 0 {
			return nil, ErrLoopbackBackend
		}
	}
	return addresses, nil
}

// Attach creates slot's namespace and veth pair, gives both ends their addresses
// with IPv6 off, routes the sandbox through the host end, and admits the slot's
// pair to the firewall. A kernel without IPv6 has none to turn off on either
// end.
func (n *Network) Attach(ctx context.Context, slot Slot) error {
	namespace := slot.Namespace()
	host := slot.HostInterface()
	peer := slot.PeerInterface()
	if err := createNamespace(namespace); err != nil {
		return err
	}
	file := namespacePath(namespace)
	opened, err := os.Open(file)
	if err != nil {
		return err
	}
	defer opened.Close()
	if err := addVeth(host, peer, slot.Gateway, opened); err != nil {
		return err
	}
	ipv6 := true
	if _, err := os.Stat(ipv6Settings); errors.Is(err, os.ErrNotExist) {
		ipv6 = false
	} else if err != nil {
		return err
	} else if err := os.WriteFile(ipv6Settings+"/conf/"+host+"/disable_ipv6", []byte("1"), 0o644); err != nil {
		return err
	}
	err = linux.InNetworkNamespace(file, func() error {
		if ipv6 {
			if err := os.WriteFile(ipv6Settings+"/conf/all/disable_ipv6", []byte("1"), 0o644); err != nil {
				return err
			}
		}
		return configurePeer(peer, slot.Address, slot.Gateway)
	})
	if err != nil {
		return err
	}
	return n.apply(ctx, AddSlotTransaction(slot))
}

// Detach removes slot's firewall pair, veth pair and namespace, each only if it
// exists.
func (n *Network) Detach(ctx context.Context, slot Slot) error {
	if err := n.apply(ctx, RemoveSlotTransaction(slot)); err != nil {
		return err
	}
	if err := deleteLink(slot.HostInterface()); err != nil {
		return err
	}
	return removeNamespace(slot.Namespace())
}
