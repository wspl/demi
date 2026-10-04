package cmdpkgs

import (
	"context"
	"errors"
	"path/filepath"
	"runtime"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/commandwire/commandwiretest"
	"github.com/wspl/demi/internal/programtest"
	"github.com/wspl/demi/internal/runner/cmdpkgs/cmdpkgstest"
)

type startingResolver struct {
	path             string
	entered, proceed chan struct{}
	calls            atomic.Int32
}

func (r *startingResolver) Resolve(ctx context.Context, _ commandwire.PackageArtifact) (ArtifactSource, error) {
	r.calls.Add(1)
	close(r.entered)
	select {
	case <-ctx.Done():
		return ArtifactSource{}, ctx.Err()
	case <-r.proceed:
		return ArtifactSource{Path: r.path}, nil
	}
}

// This test reads the registry admission count to wait for the second caller's
// lease: the registry offers no event for an admission, so the test yields
// between reads. IO and assertions use the public API.
func TestCallerGivingUpLeavesSharedStartToOthers(t *testing.T) {
	ctx := t.Context()
	path, err := programtest.Path(ctx, "demi-native-fixture")
	if err != nil {
		t.Fatal(err)
	}
	digest, err := artifacts.DigestFile(ctx, path, ^uint64(0))
	if err != nil {
		t.Fatal(err)
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	descriptor := commandwire.PackageDescriptor{
		ID:              "demi.fixture",
		Version:         "1",
		ProtocolVersion: 1,
		Operations:      commandwiretest.FixtureOperations(),
		Targets: map[string]commandwire.PackageArtifact{
			string(target): {SHA256: digest.SHA256, Size: digest.Size},
		},
	}
	root := t.TempDir()
	registry, err := NewServiceRegistry(ctx, filepath.Join(root, "cache"), "", root, map[string]string{})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := registry.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	resolver := &startingResolver{path: path, entered: make(chan struct{}), proceed: make(chan struct{})}
	impatient, giveUp := context.WithCancel(ctx)
	defer giveUp()
	first := make(chan error, 1)
	go func() {
		_, err := registry.Acquire(impatient, descriptor, resolver, cmdpkgstest.NoNumbers{})
		first <- err
	}()
	<-resolver.entered
	type result struct {
		resident *Resident
		err      error
	}
	second := make(chan result, 1)
	go func() {
		resident, err := registry.Acquire(ctx, descriptor, resolver, cmdpkgstest.NoNumbers{})
		second <- result{resident, err}
	}()
	for {
		registry.mu.Lock()
		entry := registry.entries[digest.SHA256]
		admitted := entry != nil && entry.leases == 2
		registry.mu.Unlock()
		if admitted {
			break
		}
		runtime.Gosched()
	}
	giveUp()
	if err := <-first; !errors.Is(err, context.Canceled) {
		t.Fatalf("first: %v", err)
	}
	// A manifest lease keeps the successfully started service after its caller's
	// temporary startup lease ends.
	lease, err := registry.Lease(ctx, digest.SHA256)
	if err != nil {
		t.Fatal(err)
	}
	defer lease.Release()
	close(resolver.proceed)
	answer := <-second
	if answer.err != nil {
		t.Fatal(answer.err)
	}
	if _, err := answer.resident.Client().Info(ctx); err != nil {
		t.Fatal(err)
	}
	if resolver.calls.Load() != 1 {
		t.Fatalf("starts=%d", resolver.calls.Load())
	}
}
