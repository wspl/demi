package runners

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"os"
	"strconv"
	"strings"

	"github.com/gowebpki/jcs"
	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/cmdproto"
	"gocloud.dev/blob"
	"gocloud.dev/gcerrors"
	"golang.org/x/sync/errgroup"
)

// artifactStore is native publication's conditional creation and signing boundary.
type artifactStore interface {
	urlSigner
	Attributes(context.Context, string) (*blob.Attributes, error)
	WriteAll(context.Context, string, []byte, *blob.WriterOptions) error
}

type nativeUpload struct {
	file    ArtifactFile
	encoded bool
}

// publish verifies every release before the first upload and publishes immutable
// package/version mappings only after every artifact and descriptor is in place.
func publish(
	ctx context.Context,
	releases []NativeRelease,
	prefix string,
	store artifactStore,
) (*NativeCatalog, error) {
	verified, err := verifyReleases(ctx, releases, true)
	if err != nil {
		return nil, err
	}
	uploads := make(map[string]nativeUpload)
	published := make(map[string]uint64)
	for _, release := range verified {
		for _, file := range release.executables {
			if _, exists := uploads[file.Artifact.SHA256]; !exists {
				uploads[file.Artifact.SHA256] = nativeUpload{file: file, encoded: true}
			}
		}
		for _, file := range release.archives {
			if _, exists := uploads[file.Artifact.SHA256]; !exists {
				uploads[file.Artifact.SHA256] = nativeUpload{file: file}
			}
		}
	}
	group, work := errgroup.WithContext(ctx)
	group.SetLimit(4)
	for digest, upload := range uploads {
		published[digest] = upload.file.Artifact.Size
		group.Go(func() error {
			return uploadNative(work, store, prefix+"/blobs/"+digest, upload)
		})
	}
	if err := group.Wait(); err != nil {
		return nil, err
	}
	packages := make([]cmdproto.PackageDescriptor, 0, len(verified))
	for _, release := range verified {
		descriptor := release.descriptor
		if err := publishDescriptor(ctx, store, prefix, descriptor); err != nil {
			return nil, err
		}
		packages = append(packages, descriptor)
	}
	catalog, err := NewNativeCatalog(packages, &SignedArtifacts{signer: store, prefix: prefix, published: published})
	if err != nil {
		return nil, configError(err)
	}
	return catalog, nil
}

// uploadNative rechecks bytes before uploading, and never recompresses an artifact
// already present with the right metadata.
func uploadNative(ctx context.Context, store artifactStore, key string, upload nativeUpload) error {
	file := upload.file
	found, err := nativeInPlace(ctx, store, key, file.Artifact)
	if err != nil || found {
		return err
	}
	if err := ctx.Err(); err != nil {
		return publicationStoreError(err)
	}
	data, err := os.ReadFile(file.Path)
	if err != nil {
		return releaseError(file.Path, err)
	}
	verifier := artifacts.NewVerifier(artifacts.Digest{SHA256: file.Artifact.SHA256, Size: file.Artifact.Size})
	err = verifier.Update(data)
	if err == nil {
		err = verifier.Finish()
	}
	if err != nil {
		return releaseError(file.Path, err)
	}
	coding := ""
	if upload.encoded {
		data, err = artifacts.Encode(ctx, data, artifacts.Published)
		if err != nil {
			if ctx.Err() != nil {
				return publicationStoreError(ctx.Err())
			}
			return releaseError(file.Path, err)
		}
		coding = artifacts.ContentCoding
	}
	return putImmutable(ctx, store, key, data, file.Artifact, coding)
}

// putImmutable claims one native object's meaning once, checking a competing claim.
func putImmutable(
	ctx context.Context,
	store artifactStore,
	key string,
	data []byte,
	artifact cmdproto.PackageArtifact,
	coding string,
) error {
	err := store.WriteAll(
		ctx,
		key,
		data,
		&blob.WriterOptions{
			IfNotExist:      true,
			ContentType:     "application/octet-stream",
			ContentEncoding: coding,
			Metadata: map[string]string{
				"sha256": artifact.SHA256,
				"size":   strconv.FormatUint(artifact.Size, 10),
			},
		},
	)
	if err == nil {
		return nil
	}
	if gcerrors.Code(err) == gcerrors.AlreadyExists || gcerrors.Code(err) == gcerrors.FailedPrecondition {
		_, err = nativeInPlace(ctx, store, key, artifact)
		return err
	}
	return publicationStoreError(err)
}

// nativeInPlace compares the metadata describing a native object's decoded bytes.
func nativeInPlace(
	ctx context.Context,
	store artifactStore,
	key string,
	artifact cmdproto.PackageArtifact,
) (bool, error) {
	attributes, err := store.Attributes(ctx, key)
	if gcerrors.Code(err) == gcerrors.NotFound {
		return false, nil
	}
	if err != nil {
		return false, publicationStoreError(err)
	}
	if attributes.Metadata["sha256"] != artifact.SHA256 ||
		attributes.Metadata["size"] != strconv.FormatUint(artifact.Size, 10) {
		return false, fmt.Errorf("%w: %s", ErrArtifactConflict, key)
	}
	return true, nil
}

// publicationStoreError keeps interruption distinct from an object-store failure.
func publicationStoreError(err error) error {
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return errPublicationInterrupted
	}
	return fmt.Errorf("the native artifact store failed: %w", err)
}

// publishDescriptor writes a canonical descriptor before its immutable version mapping.
func publishDescriptor(
	ctx context.Context,
	store artifactStore,
	prefix string,
	descriptor cmdproto.PackageDescriptor,
) error {
	data, err := descriptor.MarshalJSON()
	if err != nil {
		return configError(err)
	}
	body, err := jcs.Transform(data)
	if err != nil {
		return configError(err)
	}
	digest, err := descriptor.Digest()
	if err != nil {
		return configError(err)
	}
	artifact := cmdproto.PackageArtifact{SHA256: digest, Size: uint64(len(body))}
	if err := putImmutable(ctx, store, prefix+"/descriptors/"+digest+".json", body, artifact, ""); err != nil {
		return err
	}
	// The key spells the version with A-Z a-z 0-9 and -_.!~*'() as they are and
	// every other byte percent-encoded, a space as %20. QueryEscape escapes !*'() and writes a space as +; undo both.
	version := strings.NewReplacer("+", "%20", "%21", "!", "%2A", "*", "%27", "'", "%28", "(", "%29", ")").
		Replace(url.QueryEscape(descriptor.Version))
	if err := putImmutable(
		ctx,
		store,
		prefix+"/packages/"+descriptor.ID+"/"+version+".json",
		body,
		artifact,
		"",
	); err != nil {
		return err
	}
	return nil
}
