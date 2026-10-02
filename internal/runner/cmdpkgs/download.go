package cmdpkgs

import (
	"context"
	"errors"
	"io"
	"os"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandwire"
)

// fetch verifies bytes while reporting the download. Only an expired URL is refreshed.
func (c *ArtifactCache) fetch(ctx context.Context, artifact commandwire.PackageArtifact, output io.Writer, resolver ArtifactResolver, installing *installing) error {
	refreshed := false
	for {
		source, err := resolver.Resolve(ctx, artifact)
		if err != nil {
			return err
		}
		measured := &downloadProgress{output: output, installing: installing}
		if source.Path != "" {
			input, err := os.Open(source.Path)
			if err != nil {
				return err
			}
			err = artifacts.Copy(ctx, input, digestOf(artifact), measured)
			return artifactError(errors.Join(err, input.Close()))
		}
		expired := func() bool { return source.ExpiresAt != nil && !source.ExpiresAt.After(time.Now()) }
		if expired() {
			if refreshed {
				return &RuntimeError{Kind: LocationFailure, Detail: "the backend returned an expired URL"}
			}
			refreshed = true
			continue
		}
		err = artifacts.Download(ctx, c.http, source.URL, digestOf(artifact), measured)
		var rejected *artifacts.RejectedError
		if errors.As(err, &rejected) && rejected.Status == 403 && !refreshed && expired() {
			refreshed = true
			continue
		}
		return artifactError(err)
	}
}

type downloadProgress struct {
	output     io.Writer
	installing *installing
	written    uint64
}

func (w *downloadProgress) Write(b []byte) (int, error) {
	n, err := w.output.Write(b)
	w.written += uint64(n)
	w.installing.downloaded(w.written)
	return n, err
}
