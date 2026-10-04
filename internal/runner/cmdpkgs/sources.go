package cmdpkgs

import (
	"context"
	"path/filepath"
	"time"

	"github.com/wspl/demi/internal/cmdproto"
)

// ArtifactSource is a local path or an HTTP location and its optional expiry.
// A nonempty Path selects a local file; otherwise URL selects the download.
type ArtifactSource struct {
	// Path selects a local artifact when nonempty.
	Path string
	// URL selects the download when Path is empty.
	URL string
	// ExpiresAt is the optional download URL expiry.
	ExpiresAt *time.Time
}

// SourceFromLocation checks a backend location against this machine's path rules.
func SourceFromLocation(location cmdproto.ArtifactLocation) (ArtifactSource, error) {
	switch loc := location.(type) {
	case *cmdproto.ArtifactPath:
		if !filepath.IsAbs(loc.Path) {
			return ArtifactSource{}, &RuntimeError{
				Kind:   LocationFailure,
				Detail: "local artifact path must be absolute",
			}
		}
		return ArtifactSource{Path: loc.Path}, nil
	case *cmdproto.ArtifactURL:
		source := ArtifactSource{URL: loc.URL}
		if loc.ExpiresAt != nil {
			if *loc.ExpiresAt < 0 {
				return ArtifactSource{}, &RuntimeError{Kind: LocationFailure, Detail: "invalid artifact URL expiry"}
			}
			expiry := time.UnixMilli(*loc.ExpiresAt)
			source.ExpiresAt = &expiry
		}
		return source, nil
	}
	return ArtifactSource{}, &RuntimeError{Kind: LocationFailure, Detail: "missing artifact location"}
}

// ArtifactResolver resolves only artifacts authorized by the calling registration's
// catalog. URLs are resolved again for each attempt and never become cache keys.
type ArtifactResolver interface {
	// Resolve locates an artifact authorized by the caller.
	Resolve(ctx context.Context, artifact cmdproto.PackageArtifact) (ArtifactSource, error)
}

// NumberSource routes conversation numbers to the backend connection under which
// the service started. Losing that connection also ends the service.
type NumberSource interface {
	// Reserve returns the first of count numbers of the conversation's sequence.
	Reserve(
		ctx context.Context,
		conversation string,
		sequence cmdproto.ServiceSequence,
		count uint32,
	) (uint64, error)
}
