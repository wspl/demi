package cmdpkgs

import (
	"context"
	"fmt"
	"sync"

	"github.com/wspl/demi/internal/commandwire"
)

type serviceArtifacts struct {
	registry *ServiceRegistry
	pkg      string
	mu       sync.Mutex
	held     []*Hold
}

func (a *serviceArtifacts) close() {
	for _, hold := range a.held {
		hold.Release()
	}
}
func (a *serviceArtifacts) answer(ctx context.Context, request commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
	r := a.registry
	if request.Installed != nil {
		installed, err := r.cache.Installed(ctx, a.pkg, request.Installed.Name)
		return commandwire.ArtifactAnswer{ID: request.ID, Installed: &installed}, err
	}
	install := request.Install
	r.mu.Lock()
	invocation := r.invocations[install.Invocation]
	r.mu.Unlock()
	if invocation == nil || invocation.pkg != a.pkg {
		return commandwire.ArtifactAnswer{}, fmt.Errorf("%s is no running invocation of %s", install.Invocation, a.pkg)
	}
	ctx, cancel := context.WithCancel(ctx)
	cancelled := make(chan struct{})
	stop := context.AfterFunc(invocation.ctx, func() {
		cancel()
		close(cancelled)
	})
	defer func() {
		if !stop() {
			<-cancelled
		}
	}()
	defer cancel()
	hold := r.cache.Holds().Hold(install.SHA256)
	wanted := Wanted{Package: a.pkg, Name: install.Name, Version: install.Version, Artifact: commandwire.PackageArtifact{SHA256: install.SHA256, Size: install.Size}, Form: install.Form}
	path, err := r.cache.Install(ctx, wanted, invocation.resolver)
	if err != nil {
		hold.Release()
		return commandwire.ArtifactAnswer{}, err
	}
	a.mu.Lock()
	a.held = append(a.held, hold)
	a.mu.Unlock()
	return commandwire.ArtifactAnswer{ID: request.ID, Path: &path}, nil
}
