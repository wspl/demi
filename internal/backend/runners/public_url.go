package runners

import (
	"log/slog"
	"net/netip"
	"net/url"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/internal/runnerwire"
)

// PublicURL holds DEMI_BACKEND_PUBLIC_URL or the listener's address in tests.
// The edge sets it before serving; Cloud boots, exposes and development downloads
// use it. Its zero value is ready to use. Share its pointer; do not copy it.
type PublicURL struct {
	once  sync.Once
	value atomic.Pointer[runnerwire.BackendURL]
}

// Listening sets the URL once: public, or the listener's address when public is
// nil. Unspecified addresses become loopback. Invalid URLs are logged and leave
// the value unset; later calls cannot replace a URL already set.
func (p *PublicURL) Listening(public *url.URL, address netip.AddrPort) {
	text := ""
	if public != nil {
		text = public.String()
	} else {
		if address.Addr().IsUnspecified() {
			loopback := netip.IPv6Loopback()
			if address.Addr().Is4() {
				loopback = netip.AddrFrom4([4]byte{127, 0, 0, 1})
			}
			address = netip.AddrPortFrom(loopback, address.Port())
		}
		text = "http://" + address.String()
	}
	value, err := runnerwire.ParseBackendURL(text)
	if err != nil {
		slog.Error("the URL runners connect to is not usable: " + err.Error())
		return
	}
	p.once.Do(func() { p.value.Store(&value) })
}

// URL returns the backend URL once it listens, or false before it is set.
func (p *PublicURL) URL() (runnerwire.BackendURL, bool) {
	value := p.value.Load()
	if value == nil {
		return runnerwire.BackendURL{}, false
	}
	return *value, true
}
