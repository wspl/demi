package remotehost

import (
	"context"
	"errors"
	"log/slog"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/runnerproto"
)

// task admits and joins connection-owned policy work without holding the state lock.
func (l *Link) task(run func()) {
	l.mu.Lock()
	if l.endReason != "" || l.IsClosed() {
		l.mu.Unlock()
		return
	}
	l.workers.Add(1)
	l.mu.Unlock()
	go func() {
		defer l.workers.Done()
		run()
	}()
}

// growVolume returns the policy's managed-volume decision to the runner.
func (l *Link) growVolume(request *runnerproto.VolumeGrow) {
	l.task(func() {
		answer := &runnerproto.VolumeGrown{ID: request.ID, Volume: request.Volume, Bytes: request.Bytes}
		if err := l.policy.GrowVolume(l.ctx, request.Volume, request.Bytes); err != nil {
			answer.Error = new(err.Error())
		}
		if err := l.send(l.ctx, answer); err != nil {
			slog.Warn("volume growth answer not sent: "+err.Error(), "device", l.device)
		}
	})
}

// reserveNumbers bounds outstanding reservations and refuses duplicate request IDs.
func (l *Link) reserveNumbers(request *runnerproto.NumbersReserve) {
	l.mu.Lock()
	_, duplicate := l.numbers[request.ID]
	admitted := len(l.numbers) < 32 && !duplicate
	if admitted {
		l.numbers[request.ID] = struct{}{}
	}
	l.mu.Unlock()
	l.task(func() {
		answer := &runnerproto.NumbersReserved{ID: request.ID}
		if !admitted {
			answer.Error = new("Numbers request limit or duplicate id")
		} else {
			first, err := l.policy.ReserveNumbers(l.ctx, request.ConversationID, request.Sequence, request.Count)
			l.mu.Lock()
			delete(l.numbers, request.ID)
			l.mu.Unlock()
			if err != nil {
				answer.Error = new(err.Error())
			} else {
				answer.First = new(first)
			}
		}
		if err := l.send(l.ctx, answer); err != nil {
			slog.Warn("numbers answer not sent: "+err.Error(), "device", l.device)
		}
	})
}

// artifactGrant ties package downloads to one live job or service invocation.
type artifactGrant struct {
	ctx      context.Context
	resolver ArtifactResolver
	packages []commandproto.PackageDescriptor
	attached []AttachedArtifact
}

// grant snapshots the packages whose artifacts live work is allowed to install.
func (l *Link) grant(owner runnerproto.ArtifactOwner) *artifactGrant {
	l.mu.Lock()
	defer l.mu.Unlock()
	switch owner := owner.(type) {
	case *runnerproto.JobArtifactOwner:
		job := l.jobs[owner.JobID]
		if job == nil || job.commands == nil || job.commands.Hash() != owner.ManifestHash {
			return nil
		}
		grant := &artifactGrant{ctx: job.ctx, resolver: job.commands.resolver}
		for _, descriptor := range job.commands.manifest.Packages {
			grant.packages = append(grant.packages, descriptor)
		}
		return grant
	case *runnerproto.StreamArtifactOwner:
		stream := l.services[owner.StreamID]
		if stream == nil || stream.ctx.Err() != nil {
			return nil
		}
		return &artifactGrant{
			ctx:      stream.ctx,
			resolver: stream.request.Resolver,
			packages: []commandproto.PackageDescriptor{stream.request.Package},
			attached: stream.request.Attached,
		}
	}
	return nil
}

// resolveArtifact limits concurrent downloads to artifacts pinned by their live owner.
func (l *Link) resolveArtifact(request *runnerproto.ArtifactResolve) {
	grant := l.grant(request.Owner)
	refusal := ""
	if grant == nil {
		refusal = "No matching live job or stream"
	} else {
		l.mu.Lock()
		_, duplicate := l.artifacts[request.ID]
		if len(l.artifacts) >= 32 || duplicate {
			refusal = "Artifact resolution request limit or duplicate id"
		} else {
			l.artifacts[request.ID] = struct{}{}
		}
		l.mu.Unlock()
	}
	l.task(func() {
		if refusal != "" {
			l.answerArtifact(request.ID, nil, errors.New(refusal))
			return
		}
		location, err := grant.resolve(request.SHA256, request.Target)
		l.mu.Lock()
		delete(l.artifacts, request.ID)
		l.mu.Unlock()
		if grant.ctx.Err() == nil {
			l.answerArtifact(request.ID, location, err)
		}
	})
}

// resolve locates only an attached artifact or one carried by the work's packages.
func (g *artifactGrant) resolve(digest, target string) (commandproto.ArtifactLocation, error) {
	for _, attached := range g.attached {
		if attached.Artifact.SHA256 == digest {
			if err := commandproto.ValidateArtifactLocation(attached.Location); err != nil {
				return nil, err
			}
			return attached.Location, nil
		}
	}
	for _, descriptor := range g.packages {
		if artifact, ok := descriptor.Carries(commandproto.TargetTriple(target), digest); ok {
			location, err := g.resolver.Resolve(g.ctx, artifact, target)
			if err != nil {
				return nil, err
			}
			if err := commandproto.ValidateArtifactLocation(location); err != nil {
				return nil, err
			}
			return location, nil
		}
	}
	//nolint:staticcheck // ST1005: protocol refusal text, sent to the runner as it is.
	return nil, errors.New("Artifact does not belong to the live work's packages")
}

// answerArtifact sends a validated location or the refusal's text, and logs a send that fails.
func (l *Link) answerArtifact(id string, location commandproto.ArtifactLocation, err error) {
	answer := &runnerproto.ArtifactLocation{ID: id}
	if err != nil {
		answer.Error = new(err.Error())
	} else {
		answer.Location = new(location)
	}
	if err := l.send(l.ctx, answer); err != nil {
		slog.Warn("artifact location not sent: "+err.Error(), "device", l.device)
	}
}
