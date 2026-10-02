package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

import (
	"context"
	"io"
	"time"

	"github.com/wspl/demi/internal/runnerwire"
)

// PipeClient carries file contents and output through backend pipe routes.
// Its owner calls Close after cancelling and joining all pipe operations.
type PipeClient struct{}

// NewPipeClient uses a 15-second connection timeout. Token returns the current
// registration credential and its presence; it must be safe for concurrent calls.
func NewPipeClient(backend runnerwire.BackendURL, token func() (runnerwire.DeviceToken, bool)) (*PipeClient, error) {
	panic("not written: r-process")
}

// NewPipeClientWithConnectTimeout bounds connection opening only. Requests,
// answers and bodies may remain quiet for as long as their contexts allow.
func NewPipeClientWithConnectTimeout(backend runnerwire.BackendURL, token func() (runnerwire.DeviceToken, bool), timeout time.Duration) (*PipeClient, error) {
	panic("not written: r-process")
}

// Open reads an origin-relative pipe route. The caller closes the body; its
// context remains active for the body's entire lifetime.
func (c *PipeClient) Open(ctx context.Context, path string) (io.ReadCloser, error) {
	panic("not written: r-process")
}

// Put sends an origin-relative pipe route and reads its bounded confirmation.
// It takes ownership of body and closes it on every outcome. Close must unblock Read.
func (c *PipeClient) Put(ctx context.Context, path string, body io.ReadCloser) error {
	panic("not written: r-process")
}

// Close releases idle transport connections after operations have finished.
func (c *PipeClient) Close() error { panic("not written: r-process") }

// ReportPipe encodes and sends a pipe outcome; cancellation ends the wait when
// the backend is gone. Output carries encoded runner wire frames.
func ReportPipe(ctx context.Context, output chan<- []byte, id string, result error) error {
	panic("not written: r-process")
}
