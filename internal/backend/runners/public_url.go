package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"net/netip"
	"net/url"

	"github.com/wspl/demi/internal/runnerwire"
)

// PublicURL holds DEMI_BACKEND_PUBLIC_URL or the listener's address in tests.
// The edge sets it before serving; Cloud boots, exposes and development downloads
// use it. Its zero value is ready to use. Share its pointer; do not copy it.
type PublicURL struct{}

// Listening sets the URL once: public, or the listener's address when public is
// nil. Unspecified addresses become loopback. Invalid URLs are logged and leave
// the value unset; later calls cannot replace a URL already set.
func (p *PublicURL) Listening(public *url.URL, address netip.AddrPort) {
	panic("not written: b-runners")
}

// URL returns the backend URL once it listens, or false before it is set.
func (p *PublicURL) URL() (runnerwire.BackendURL, bool) { panic("not written: b-runners") }
