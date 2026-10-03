package edge

import (
	"context"
	"net"
	"net/netip"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/core"
)

// The backend listens on every IPv4 address (0.0.0.0) and on no IPv6 one;
// Go's "tcp" network would add IPv6.
func TestUnspecifiedIPv4ListensOnIPv4Only(t *testing.T) {
	control, err := database.OpenControl(t.Context(), filepath.Join(t.TempDir(), "control.sqlite"), core.SystemClock{})
	if err != nil {
		t.Fatal(err)
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = control.Close(context.Background()) }()
	state := AppState{Services: &usershard.Services{Control: control, PublicURL: &runners.PublicURL{}}, Site: &Site{}}
	edge, err := Start(t.Context(), netip.MustParseAddrPort("0.0.0.0:0"), state, "")
	if err != nil {
		t.Fatal(err)
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = edge.Close(context.Background()) }()
	port := edge.LocalAddr().Port()
	dialer := net.Dialer{}
	ipv4, err := dialer.DialContext(
		t.Context(),
		"tcp",
		netip.AddrPortFrom(netip.MustParseAddr("127.0.0.1"), port).String(),
	)
	if err != nil {
		t.Fatal("IPv4 loopback refused:", err)
	}
	_ = ipv4.Close()
	ipv6, err := dialer.DialContext(t.Context(), "tcp", netip.AddrPortFrom(netip.IPv6Loopback(), port).String())
	if err == nil {
		_ = ipv6.Close()
		t.Fatal("the IPv4 listener accepted an IPv6 connection")
	}
}
