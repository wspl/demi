//go:build linux

package network

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"net/url"
	"os"
	"slices"
	"strconv"
	"strings"

	"github.com/vishvananda/netlink"
	"github.com/wspl/demi/internal/machinemanager/system"
	"golang.org/x/sys/unix"
)

// Network is the host side of Cloud networking. Configuration is immutable;
// the manager serializes Prepare with slot operations and operations on the
// same slot. Construct it with New.
type Network struct {
	pool    netip.Prefix
	dns     []netip.Addr
	backend url.URL
}

// New creates the host network policy from the validated IPv4 pool, IPv4 DNS
// resolvers and HTTP(S) backend URL. It copies the supplied configuration.
// Firewall transactions use nftables directly with an explicit namespace FD.
func New(pool netip.Prefix, dns []netip.Addr, backend url.URL) *Network {
	return &Network{pool: pool, dns: slices.Clone(dns), backend: backend}
}

// Prepare checks that the pool overlaps no host route, resolves the backend,
// enables forwarding, and replaces the firewall table. Recovery calls this
// after detaching recorded slots; replacement starts with an empty slot set.
func (n *Network) Prepare(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	handle, err := netlink.NewHandle(unix.NETLINK_ROUTE)
	if err != nil {
		return err
	}
	defer handle.Close()
	if err := n.checkRoutes(ctx, handle); err != nil {
		return err
	}
	backend, err := n.backendAddresses(ctx)
	if err != nil {
		return err
	}
	if err := os.WriteFile("/proc/sys/net/ipv4/ip_forward", []byte("1"), 0o644); err != nil {
		return err
	}
	port := uint16(80)
	if n.backend.Scheme == "https" {
		port = 443
	}
	if text := n.backend.Port(); text != "" {
		parsed, err := strconv.ParseUint(text, 10, 16)
		if err != nil {
			return err
		}
		port = uint16(parsed)
	}
	return applyFirewall(ctx, func(batch *firewallBatch) error { return batch.prepare(n.pool, backend, port, n.dns) })
}

// Attach creates slot's namespace and veth pair, gives both ends their
// addresses with IPv6 off, routes the sandbox through the host end, and admits
// the slot's pair to the firewall. A kernel without IPv6 has none to turn off.
// On failure the owner must call Detach with a cleanup context to remove any
// partially created resources before releasing the slot lease.
func (n *Network) Attach(ctx context.Context, slot Slot) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	path := namespacePath(slot)
	if err := createNamespace(ctx, path); err != nil {
		return err
	}
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	// The descriptor only keeps the namespace alive for this operation.
	defer func() { _ = file.Close() }()
	handle, err := netlink.NewHandle(unix.NETLINK_ROUTE)
	if err != nil {
		return err
	}
	defer handle.Close()
	if err := handle.LinkAdd(
		&netlink.Veth{LinkAttrs: netlink.LinkAttrs{Name: slot.HostInterface()}, PeerName: slot.PeerInterface()},
	); err != nil {
		return err
	}
	peer, err := handle.LinkByName(slot.PeerInterface())
	if err != nil {
		return err
	}
	if err := handle.LinkSetNsFd(peer, int(file.Fd())); err != nil {
		return err
	}
	if err := addressAndUp(ctx, handle, slot.HostInterface(), slot.Gateway); err != nil {
		return err
	}
	_, err = os.Stat(ipv6Settings)
	ipv6 := err == nil
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	if ipv6 {
		if err := os.WriteFile(
			ipv6Settings+"/conf/"+slot.HostInterface()+"/disable_ipv6",
			[]byte("1"),
			0o644,
		); err != nil {
			return err
		}
	}
	// A descriptor path also works when the caller has an isolated mount namespace.
	_, err = system.RunNamespace(
		ctx,
		system.Network(fmt.Sprintf("/proc/self/fd/%d", file.Fd())),
		func(ctx context.Context) (struct{}, error) {
			return struct{}{}, configurePeer(ctx, slot, ipv6)
		},
	)
	if err != nil {
		return err
	}
	return applyFirewall(ctx, func(batch *firewallBatch) error { return batch.attach(slot) })
}

// Detach removes slot's firewall pair, veth pair and namespace, each only if
// it exists. Recovery may pass a slot reconstructed from a saved record.
// Cleanup callers use a context without cancellation and await completion.
func (n *Network) Detach(ctx context.Context, slot Slot) error {
	if err := applyFirewall(ctx, func(batch *firewallBatch) error { return batch.detach(slot) }); err != nil {
		return err
	}
	handle, err := netlink.NewHandle(unix.NETLINK_ROUTE)
	if err != nil {
		return err
	}
	defer handle.Close()
	link, err := handle.LinkByName(slot.HostInterface())
	var absent netlink.LinkNotFoundError
	if err != nil && !errors.As(err, &absent) && !errors.Is(err, unix.ENODEV) {
		return err
	}
	if err == nil {
		if err := handle.LinkDel(link); err != nil && !errors.Is(err, unix.ENODEV) {
			return err
		}
	}
	return removeNamespace(ctx, namespacePath(slot))
}

const ipv6Settings = "/proc/sys/net/ipv6"

// backendAddresses resolves the Cloud runner destination and rejects host loopback.
func (n *Network) backendAddresses(ctx context.Context) ([]netip.Addr, error) {
	host := n.backend.Hostname()
	var addresses []netip.Addr
	if literal, err := netip.ParseAddr(host); err == nil {
		if literal.Is4() {
			addresses = append(addresses, literal)
		}
	} else {
		resolved, err := net.DefaultResolver.LookupNetIP(ctx, "ip", host)
		if err != nil {
			return nil, err
		}
		for _, address := range resolved {
			if address.Unmap().Is4() {
				addresses = append(addresses, address.Unmap())
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

// addressAndUp assigns the Cloud end of a /30 and enables its interface.
func addressAndUp(ctx context.Context, handle *netlink.Handle, name string, address netip.Addr) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	link, err := handle.LinkByName(name)
	if err != nil {
		return err
	}
	if err := handle.AddrAdd(
		link,
		&netlink.Addr{IPNet: &net.IPNet{IP: net.IP(address.AsSlice()), Mask: net.CIDRMask(30, 32)}},
	); err != nil {
		return err
	}
	return handle.LinkSetUp(link)
}

// checkRoutes borrows the route handle; Prepare retains it through firewall installation.
func (n *Network) checkRoutes(_ context.Context, handle *netlink.Handle) error {
	routes, err := handle.RouteListFiltered(
		netlink.FAMILY_V4,
		&netlink.Route{Table: unix.RT_TABLE_MAIN},
		netlink.RT_FILTER_TABLE,
	)
	if err != nil {
		return err
	}
	links, err := handle.LinkList()
	if err != nil {
		return err
	}
	names := make(map[int]string, len(links))
	for _, link := range links {
		names[link.Attrs().Index] = link.Attrs().Name
	}
	for _, route := range routes {
		if route.Dst == nil || strings.HasPrefix(names[route.LinkIndex], hostInterfacePrefix) {
			continue
		}
		bits, _ := route.Dst.Mask.Size()
		if bits == 0 {
			continue
		}
		address, ok := netip.AddrFromSlice(route.Dst.IP)
		if !ok {
			return fmt.Errorf("invalid IPv4 route destination: %w", unix.EINVAL)
		}
		prefix := netip.PrefixFrom(address.Unmap(), bits).Masked()
		if n.pool.Overlaps(prefix) {
			return fmt.Errorf("DEMI_MANAGED_SUBNET overlaps host route %s", prefix)
		}
	}
	return nil
}

func configurePeer(ctx context.Context, slot Slot, ipv6 bool) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if ipv6 {
		if err := os.WriteFile(ipv6Settings+"/conf/all/disable_ipv6", []byte("1"), 0o644); err != nil {
			return err
		}
	}
	inside, err := netlink.NewHandle(unix.NETLINK_ROUTE)
	if err != nil {
		return err
	}
	defer inside.Close()
	loopback, err := inside.LinkByName("lo")
	if err != nil {
		return err
	}
	if err := inside.LinkSetUp(loopback); err != nil {
		return err
	}
	if err := addressAndUp(ctx, inside, slot.PeerInterface(), slot.Address); err != nil {
		return err
	}
	return inside.RouteAdd(&netlink.Route{Gw: net.IP(slot.Gateway.AsSlice())})
}
