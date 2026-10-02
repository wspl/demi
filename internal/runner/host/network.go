package host

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
)

// NetOpen resolves the host on the device and connects with the Rust ten-second
// connection timeout. It answers net_opened before moving bytes, then joins both
// pipe directions. Input EOF half-closes the socket; socket EOF ends output;
// either pipe failing cancels both directions. It reports both pipe ends even
// when connecting fails. Streams do not consume finite Host-work permits.
func (s *Service) NetOpen(ctx context.Context, request runnerwire.NetOpen) error {
	panic("not written: r-host")
}
