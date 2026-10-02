package jobs

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runnerwire"
)

// jobArtifacts locates an artifact through any live job that authorizes it.
type jobArtifacts struct{ contexts *Contexts }

func (r *jobArtifacts) Resolve(ctx context.Context, artifact commandwire.PackageArtifact) (cmdpkgs.ArtifactSource, error) {
	for {
		execution, ok := r.contexts.Carrying(artifact.SHA256)
		if !ok {
			return cmdpkgs.ArtifactSource{}, &cmdpkgs.RuntimeError{Kind: cmdpkgs.LocationFailure, Detail: "no live job authorizes this artifact"}
		}
		wait, cancel := context.WithCancel(ctx)
		done := make(chan struct{})
		go func() {
			defer close(done)
			select {
			case <-execution.Done():
				cancel()
			case <-wait.Done():
			}
		}()
		location, err := execution.Connection.Locate(wait, &runnerwire.JobArtifactOwner{JobID: execution.JobID, ManifestHash: execution.Manifest.Hash}, artifact.SHA256)
		cancel()
		<-done
		if ctx.Err() != nil {
			return cmdpkgs.ArtifactSource{}, ctx.Err()
		}
		if execution.lifetime.Err() != nil {
			continue
		}
		if err != nil {
			return cmdpkgs.ArtifactSource{}, err
		}
		return cmdpkgs.SourceFromLocation(location)
	}
}

// streamArtifacts locates artifacts authorized by one open user stream.
type streamArtifacts struct {
	connection *ConnectionHandle
	stream     string
}

func (r *streamArtifacts) Resolve(ctx context.Context, artifact commandwire.PackageArtifact) (cmdpkgs.ArtifactSource, error) {
	location, err := r.connection.Locate(ctx, &runnerwire.StreamArtifactOwner{StreamID: r.stream}, artifact.SHA256)
	if err != nil {
		return cmdpkgs.ArtifactSource{}, err
	}
	return cmdpkgs.SourceFromLocation(location)
}
