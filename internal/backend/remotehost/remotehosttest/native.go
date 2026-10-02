package remotehosttest

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"slices"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/commandwire/commandwiretest"
)

// NativeFixture is a built native command package and its executable location.
type NativeFixture struct {
	Descriptor commandwire.PackageDescriptor
	path       string
}

// LoadNativeFixture resolves and describes the runner's native test package.
func LoadNativeFixture(ctx context.Context) (*NativeFixture, error) {
	path, err := NativeFixtureBinary(ctx)
	if err != nil {
		return nil, err
	}
	return NewNativeFixture(ctx, "demicodes.runner-test", path, commandwiretest.FixtureOperations())
}

// NewNativeFixture describes the package served by path for this machine's target.
func NewNativeFixture(ctx context.Context, id, path string, operations []string) (*NativeFixture, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		return nil, err
	}
	digest := sha256.Sum256(data)
	descriptor := commandwire.PackageDescriptor{ID: id, Version: "test", ProtocolVersion: 1, Operations: slices.Clone(operations), Targets: map[string]commandwire.PackageArtifact{string(target): {SHA256: hex.EncodeToString(digest[:]), Size: uint64(len(data))}}}
	if err := descriptor.Validate(); err != nil {
		return nil, err
	}
	return &NativeFixture{Descriptor: descriptor, path: path}, nil
}

// Resolver resolves the package executable to its path on this machine.
func (f *NativeFixture) Resolver() remotehost.ArtifactResolver {
	return localArtifact{path: f.path, descriptor: f.Descriptor}
}

// localArtifact locates only the native fixture's declared executable.
type localArtifact struct {
	path       string
	descriptor commandwire.PackageDescriptor
}

func (a localArtifact) Resolve(ctx context.Context, artifact commandwire.PackageArtifact, _ string) (commandwire.ArtifactLocation, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	found := false
	for _, expected := range a.descriptor.Targets {
		if expected == artifact {
			found = true
			break
		}
	}
	if !found {
		return nil, errors.New("the artifact is not the fixture's")
	}
	return &commandwire.ArtifactPath{Path: a.path}, nil
}
