//go:build linux

package network

import (
	"errors"
	"net"
	"net/netip"
	"os"

	"github.com/vishvananda/netlink"
	"golang.org/x/sys/unix"
)

// A route is an IPv4 route of the main table that names a destination, with the
// name of its output interface.
type route struct {
	network    netip.Prefix
	interface_ string
}

// mainRoutes returns the IPv4 routes of the main table that name a destination.
func mainRoutes() ([]route, error) {
	links, err := netlink.LinkList()
	if err != nil {
		return nil, err
	}
	names := make(map[int]string, len(links))
	for _, link := range links {
		names[link.Attrs().Index] = link.Attrs().Name
	}
	found, err := netlink.RouteListFiltered(netlink.FAMILY_V4, &netlink.Route{Table: unix.RT_TABLE_MAIN}, netlink.RT_FILTER_TABLE)
	if err != nil {
		return nil, err
	}
	var routes []route
	for _, entry := range found {
		if entry.Dst == nil {
			continue
		}
		address, ok := netip.AddrFromSlice(entry.Dst.IP)
		if !ok {
			continue
		}
		ones, _ := entry.Dst.Mask.Size()
		routes = append(routes, route{
			network:    netip.PrefixFrom(address.Unmap(), ones).Masked(),
			interface_: names[entry.LinkIndex],
		})
	}
	return routes, nil
}

// linkByName returns the interface name, or nil when there is none.
func linkByName(name string) (netlink.Link, error) {
	link, err := netlink.LinkByName(name)
	var missing netlink.LinkNotFoundError
	switch {
	case err == nil:
		return link, nil
	case errors.As(err, &missing), errors.Is(err, unix.ENODEV):
		return nil, nil
	}
	return nil, err
}

// existing returns the interface name, which must exist.
func existing(name string) (netlink.Link, error) {
	link, err := linkByName(name)
	if err != nil {
		return nil, err
	}
	if link == nil {
		return nil, &os.PathError{Op: "no interface", Path: name, Err: os.ErrNotExist}
	}
	return link, nil
}

// addVeth creates the veth pair host, peer in the calling thread's namespace,
// moves peer into the namespace file refers to, and gives host the address
// gateway/30 and brings it up.
func addVeth(host, peer string, gateway netip.Addr, namespace *os.File) error {
	veth := &netlink.Veth{LinkAttrs: netlink.LinkAttrs{Name: host}, PeerName: peer}
	if err := netlink.LinkAdd(veth); err != nil {
		return err
	}
	peerLink, err := existing(peer)
	if err != nil {
		return err
	}
	if err := netlink.LinkSetNsFd(peerLink, int(namespace.Fd())); err != nil {
		return err
	}
	return addressAndUp(host, gateway)
}

// addressAndUp gives the interface name the address address/30 and brings it up.
func addressAndUp(name string, address netip.Addr) error {
	link, err := existing(name)
	if err != nil {
		return err
	}
	prefix := &net.IPNet{IP: address.AsSlice(), Mask: net.CIDRMask(30, 32)}
	if err := netlink.AddrAdd(link, &netlink.Addr{IPNet: prefix}); err != nil {
		return err
	}
	return netlink.LinkSetUp(link)
}

// configurePeer brings loopback up in the calling thread's namespace, gives peer
// its address and routes everything through gateway. The calling thread is in the
// sandbox's namespace.
func configurePeer(peer string, address, gateway netip.Addr) error {
	loopback, err := existing("lo")
	if err != nil {
		return err
	}
	if err := netlink.LinkSetUp(loopback); err != nil {
		return err
	}
	if err := addressAndUp(peer, address); err != nil {
		return err
	}
	return netlink.RouteAdd(&netlink.Route{Gw: gateway.AsSlice()})
}

// deleteLink deletes the interface name if it exists; deleting one end of a veth
// pair removes both.
func deleteLink(name string) error {
	link, err := linkByName(name)
	if err != nil || link == nil {
		return err
	}
	if err := netlink.LinkDel(link); err != nil && !errors.Is(err, unix.ENODEV) {
		return err
	}
	return nil
}
