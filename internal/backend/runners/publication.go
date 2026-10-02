package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
)

// PublicationErrorKind identifies why publication stopped backend startup.
type PublicationErrorKind uint8

const (
	// PublicationConfig means DEMI_NATIVE_CONFIG cannot be used.
	PublicationConfig PublicationErrorKind = iota
	// PublicationRelease means a release cannot be published whole.
	PublicationRelease
	// PublicationConflict means an immutable object differs from the one in place.
	PublicationConflict
	// PublicationStore means the artifact store failed.
	PublicationStore
	// PublicationCancelled means publication was interrupted.
	PublicationCancelled
)

// PublicationError is a configuration, release, conflict, store or cancellation
// failure. Use errors.As to inspect Kind and errors.Is for an underlying cause.
type PublicationError struct {
	// Kind identifies the failure category.
	Kind PublicationErrorKind
	// Directory names a refused release.
	Directory string
	// Reason describes an invalid configuration or release, or names a conflicting key.
	Reason string
	// Err preserves a store, IO or context error where present.
	Err error
}

// Error returns the publication failure shown at startup.
func (e *PublicationError) Error() string { panic("not written: b-runners") }

// Unwrap returns the underlying cause.
func (e *PublicationError) Unwrap() error { panic("not written: b-runners") }

// PublishNative reads the configuration at path, verifies all named releases and
// publishes their artifacts before returning a catalog. A local store uploads
// nothing. Cancellation interrupts startup; every failure closes opened resources.
// The successful caller owns the catalog and must close it after its users stop.
func PublishNative(ctx context.Context, path string) (*NativeCatalog, error) {
	panic("not written: b-runners")
}

// SignedArtifacts resolves published artifacts with a URL signed for each request.
// PublishNative constructs it only after all artifacts and mappings are published.
type SignedArtifacts struct{}

// Resolve returns an HTTPS download signed for five minutes, only when the
// artifact's SHA-256 and size occur in the published catalog.
func (a *SignedArtifacts) Resolve(ctx context.Context, artifact commandwire.PackageArtifact, target string) (commandwire.ArtifactLocation, error) {
	panic("not written: b-runners")
}
