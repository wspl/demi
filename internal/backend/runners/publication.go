package runners

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"net/url"
	"time"

	"github.com/wspl/demi/internal/cmdproto"
	"gocloud.dev/blob"
)

// ErrArtifactConflict means an immutable native artifact object differs from the one in place.
var ErrArtifactConflict = errors.New("an immutable native artifact object differs from the one in place")

var errPublicationInterrupted = errors.New("native artifact publication was interrupted")

// configError refuses DEMI_NATIVE_CONFIG; err says why.
func configError(err error) error {
	return fmt.Errorf("DEMI_NATIVE_CONFIG cannot be used: %w", err)
}

// releaseError refuses the native release in directory; err says why.
func releaseError(directory string, err error) error {
	return fmt.Errorf("the native release in %s cannot be published: %w", directory, err)
}

// PublishNative reads the configuration at path, verifies all named releases and
// publishes their artifacts before returning a catalog. A local store uploads
// nothing. Cancellation interrupts startup; every failure closes opened resources.
// The successful caller owns the catalog and must close it after its users stop.
func PublishNative(ctx context.Context, path string) (*NativeCatalog, error) {
	config, err := readNativeConfig(ctx, path)
	if err != nil {
		return nil, err
	}
	var catalog *NativeCatalog
	switch store := config.Store.(type) {
	case *S3NativeStore:
		bucket, err := store.Open(ctx)
		if err != nil {
			return nil, publicationStoreError(err)
		}
		catalog, err = publish(ctx, config.Releases, config.prefix(), bucket)
		if err != nil {
			return nil, errors.Join(err, bucket.Close())
		}
		catalog.closeStore = bucket.Close
	case *LocalNativeStore:
		verified, err := verifyReleases(ctx, config.Releases, false)
		if err != nil {
			return nil, err
		}
		packages := make([]cmdproto.PackageDescriptor, 0, len(verified))
		var executables, archives []ArtifactFile
		for _, release := range verified {
			packages = append(packages, release.descriptor)
			executables = append(executables, release.executables...)
			archives = append(archives, release.archives...)
		}
		slog.Info("the development store serves the native releases' artifacts itself", "packages", len(packages))
		catalog, err = NewNativeCatalog(packages, NewLocalArtifacts(executables, archives))
		if err != nil {
			return nil, configError(err)
		}
	}
	return catalog, nil
}

// SignedArtifacts resolves published artifacts with a URL signed for each request.
// PublishNative constructs it only after all artifacts and mappings are published.
type SignedArtifacts struct {
	signer    urlSigner
	prefix    string
	published map[string]uint64
}

// urlSigner signs only the download operation native publication needs.
type urlSigner interface {
	SignedURL(context.Context, string, *blob.SignedURLOptions) (string, error)
}

// Resolve returns an HTTPS download signed for five minutes, only when the
// artifact's SHA-256 and size occur in the published catalog.
func (a *SignedArtifacts) Resolve(
	ctx context.Context,
	artifact cmdproto.PackageArtifact,
	_ string,
) (cmdproto.ArtifactLocation, error) {
	if err := ctx.Err(); err != nil {
		return nil, fmt.Errorf("the work that asked no longer runs: %w", err)
	}
	size, ok := a.published[artifact.SHA256]
	if !ok || size != artifact.Size {
		return nil, errors.New("the artifact is not in the published package catalog")
	}
	location, err := a.signer.SignedURL(
		ctx,
		a.prefix+"/blobs/"+artifact.SHA256,
		&blob.SignedURLOptions{Expiry: 300 * time.Second, Method: http.MethodGet},
	)
	if err != nil {
		return nil, err
	}
	parsed, err := url.Parse(location)
	if err != nil {
		return nil, err
	}
	if parsed.Scheme != "https" {
		return nil, errors.New("a native artifact downloads over HTTPS only")
	}
	expires := time.Now().Add(300 * time.Second).UnixMilli()
	return &cmdproto.ArtifactURL{URL: location, ExpiresAt: &expires}, nil
}
