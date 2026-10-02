//revive:disable:unused-parameter API checkpoint retains parameter names for callers; bodies follow after merge.

package cmdpkgs

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/commandwire"
)

// ArtifactSource is a local path or an HTTP location and its optional expiry.
// A nonempty Path selects a local file; otherwise URL selects the download.
type ArtifactSource struct {
	Path      string
	URL       string
	ExpiresAt *time.Time
}

// SourceFromLocation checks a backend location against this machine's path rules.
func SourceFromLocation(location commandwire.ArtifactLocation) (ArtifactSource, error) {
	panic("not written: r-cmdpkgs")
}

// ArtifactResolver resolves only artifacts authorized by the calling registration's
// catalog. URLs are resolved again for each attempt and never become cache keys.
type ArtifactResolver interface {
	Resolve(ctx context.Context, artifact commandwire.PackageArtifact) (ArtifactSource, error)
}

// NumberSource routes conversation numbers to the backend connection under which
// the service started. Losing that connection also ends the service.
type NumberSource interface {
	// Reserve returns the first of count numbers of the conversation's sequence.
	Reserve(ctx context.Context, conversation string, sequence commandwire.ServiceSequence, count uint32) (uint64, error)
}
