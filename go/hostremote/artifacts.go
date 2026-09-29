package hostremote

import (
	"context"
	"errors"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/runnerproto"
)

// resolveArtifact admits only packages pinned to live work on this connection.
func (l *Link) resolveArtifact(request runnerproto.OutboundArtifactResolve) {
	var packages []commandservice.PackageDescriptor
	var resolver ArtifactResolver
	var owner context.Context
	l.mu.Lock()
	switch o := request.Owner.(type) {
	case runnerproto.JobArtifactOwner:
		if job := l.jobs[o.JobID]; job != nil && job.commands != nil && job.commands.Hash() == o.ManifestHash {
			for _, p := range job.commands.manifest.Packages {
				packages = append(packages, p)
			}
			resolver, owner = job.commands.resolver, job.ctx
		}
	case runnerproto.StreamArtifactOwner:
		if service := l.services[o.StreamID]; service != nil {
			packages = []commandservice.PackageDescriptor{service.descriptor}
			resolver, owner = service.resolver, service.ctx
		}
	}
	var refusal string
	if owner == nil {
		refusal = "No matching live job or stream"
	} else if len(l.artifacts) >= ArtifactRequests || l.artifacts[request.ID] {
		refusal = "Artifact resolution request limit or duplicate id"
	} else {
		l.artifacts[request.ID] = true
	}
	l.mu.Unlock()
	l.spawn(func() {
		var location commandservice.ArtifactLocation
		var failure error
		if refusal != "" {
			failure = errors.New(refusal)
		} else {
			defer func() {
				l.mu.Lock()
				delete(l.artifacts, request.ID)
				l.mu.Unlock()
			}()
			var artifact *commandservice.PackageArtifact
			for _, p := range packages {
				if a, ok := p.Targets[commandservice.TargetTriple(request.Target)]; ok && a.SHA256 == request.SHA256 {
					artifact = &a
					break
				}
			}
			if artifact == nil {
				failure = errors.New("Artifact does not belong to the live work's packages")
			} else {
				location, failure = resolver.Resolve(owner, *artifact, request.Target)
				if failure == nil {
					failure = commandservice.Validate(location)
				}
			}
			if owner.Err() != nil {
				return
			}
		}
		var why *string
		if failure != nil {
			why = new(failure.Error())
			location = nil
		}
		// An ended connection no longer has a runner awaiting the response.
		_ = l.send(l.ctx, runnerproto.InboundArtifactLocation{ID: request.ID, Location: locationPointer(location), Error: why})
	})
}
func (l *Link) reserveNumbers(request runnerproto.OutboundNumbersReserve) {
	l.mu.Lock()
	admitted := len(l.numbers) < NumbersRequests && !l.numbers[request.ID]
	if admitted {
		l.numbers[request.ID] = true
	}
	l.mu.Unlock()
	l.spawn(func() {
		var first *uint64
		var failure error
		if !admitted {
			failure = errors.New("Numbers request limit or duplicate id")
		} else {
			value, err := l.policy.ReserveNumbers(l.ctx, request.ConversationID, request.Sequence, request.Count)
			failure = err
			if err == nil {
				first = &value
			}
			l.mu.Lock()
			delete(l.numbers, request.ID)
			l.mu.Unlock()
		}
		var why *string
		if failure != nil {
			why = new(failure.Error())
		}
		// Teardown cancels reservations and makes their answers unnecessary.
		_ = l.send(l.ctx, runnerproto.InboundNumbersReserved{ID: request.ID, First: first, Error: why})
	})
}

// locationPointer omits a refused artifact location from the runner reply.
func locationPointer(location commandservice.ArtifactLocation) *commandservice.ArtifactLocation {
	if location == nil {
		return nil
	}
	return &location
}
