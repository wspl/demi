package host

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
)

// ReadFile opens and positions the file, replies, then streams the requested
// range to its output pipe. It reports pipe_done even when opening fails.
// Pipe failure stops the read and closes the file.
func (s *Service) ReadFile(ctx context.Context, request runnerwire.FSReadFile) error {
	panic("not written: r-host")
}

// WriteFile fills an artifacts.Staged file beside the destination from the
// input pipe and publishes only after clean EOF. Failure removes the temporary
// file and preserves the destination. It reports pipe_done before the reply.
// Parent creation uses artifacts.Parent and occurs only when requested.
func (s *Service) WriteFile(ctx context.Context, request runnerwire.FSWriteFile) error {
	panic("not written: r-host")
}
