package host

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/runnerwire"
)

// MaxFiles is where the change list stops and reports truncated.
const MaxFiles = 5000

// MaxBlobBytes is the size beyond which files count no lines and GitShow refuses them.
const MaxBlobBytes = 8 * 1024 * 1024

// GitChanges reports uncommitted paths under the requested root, relative to
// that root, with status letters and HEAD-to-disk line counts. Nonrepositories
// receive an empty successful result. The service keeps at most eight watches,
// expires them after fifteen idle minutes, and shares computations per root.
// Computation has a thirty-second running deadline. Unavailable or failed
// watches cause whole walks; lost events invalidate the watched baseline.
func (s *Service) GitChanges(ctx context.Context, request runnerwire.GitChangesMessage) error {
	panic("not written: r-host")
}

// GitShow decodes a committed blob before replying and streams it whole into
// the output pipe after the reply. The working-tree permit covers decoding,
// not uploading. Oversized blobs answer too_large; missing paths answer ENOENT;
// a root outside a repository answers not_repository. Every named pipe ends
// with pipe_done, including a pipe unused because the request failed.
func (s *Service) GitShow(ctx context.Context, request runnerwire.GitShow) error {
	panic("not written: r-host")
}
