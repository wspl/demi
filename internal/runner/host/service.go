package host

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/runner/process"
)

// Service performs Host operations with one set of admission limits and watched
// working-tree baselines. It knows no jobs or backend connection. Its owner
// cancels the lifetime context and calls Close before releasing pipes or output.
// Methods are safe for concurrent use; each operation runs until its reply and
// any pipe transfers finish. The caller runs them off its control-message loop.
// Requests must already have passed the runnerwire generated decoder.
// Operations emit encoded replies and pipe_done frames to output; an operation
// failure is a wire error reply, while a returned error means delivery or
// cancellation prevented completion. The caller owns and closes output only
// after Close has joined all work.
type Service struct{}

// New creates a Host operation owner. A request without a working directory uses
// defaultCWD, which is a starting directory, never a filesystem sandbox.
// The caller retains ownership of pipes and output. Filesystem work admits 32
// requests, working-tree work eight requests and two computations; later work
// waits for admission, never fails because the Host is busy. Streams retain
// their own backpressure and do not hold finite-work permits while copying.
func New(ctx context.Context, defaultCWD string, pipes *process.PipeClient, output chan<- []byte) *Service {
	panic("not written: r-host")
}

// NewWithFileLimit creates the same owner with a working-tree list limit in
// place of MaxFiles, for callers that exercise truncation on small fixtures.
func NewWithFileLimit(ctx context.Context, defaultCWD string, pipes *process.PipeClient, output chan<- []byte, maxFiles int) *Service {
	panic("not written: r-host")
}

// Close stops admission, cancels outstanding operations and watches, and joins
// their work. It is idempotent. Use a cleanup context that remains live after
// the lifetime context is canceled; a canceled cleanup wait may be resumed by
// calling Close again. The service does not close pipes or output.
func (s *Service) Close(ctx context.Context) error {
	panic("not written: r-host")
}
