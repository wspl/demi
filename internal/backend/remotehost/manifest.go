package remotehost

import (
	"context"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
)

// ArtifactResolver locates a package executable while the requesting work lives.
type ArtifactResolver interface {
	Resolve(context.Context, cmdproto.PackageArtifact, string) (cmdproto.ArtifactLocation, error)
}

// CommandCatalog binds commands to package descriptors and executable locations.
type CommandCatalog struct {
	packages []cmdproto.PackageDescriptor
	resolver ArtifactResolver
}

// NewCommandCatalog validates descriptors and requires unique package IDs.
func NewCommandCatalog(packages []cmdproto.PackageDescriptor, resolver ArtifactResolver) (*CommandCatalog, error) {
	manifest, err := runnerproto.BuildManifest(nil, packages)
	if err != nil {
		return nil, err
	}
	data, err := manifest.MarshalJSON()
	if err != nil {
		return nil, err
	}
	owned, err := runnerproto.DecodeManifest(data)
	if err != nil {
		return nil, err
	}
	descriptors := make([]cmdproto.PackageDescriptor, 0, len(owned.Packages))
	for _, descriptor := range owned.Packages {
		descriptors = append(descriptors, descriptor)
	}
	return &CommandCatalog{packages: descriptors, resolver: resolver}, nil
}

// Select pins each native command to its descriptor in the catalog.
func (c *CommandCatalog) Select(commands *host.CommandSet) (*CommandSelection, error) {
	manifest, err := runnerproto.BuildManifest(commands.Declarations(), c.packages)
	if err != nil {
		return nil, err
	}
	wire, err := manifest.MarshalJSON()
	if err != nil {
		return nil, err
	}
	return &CommandSelection{manifest: manifest, wire: wire, resolver: c.resolver}, nil
}

// CommandSelection is a job's pinned manifest and artifact resolver.
type CommandSelection struct {
	manifest runnerproto.Manifest
	wire     []byte
	resolver ArtifactResolver
}

// Manifest returns the pinned manifest, which callers must treat as immutable.
func (s *CommandSelection) Manifest() runnerproto.Manifest {
	return s.manifest
}

// Hash returns the pinned manifest's hash.
func (s *CommandSelection) Hash() string {
	return s.manifest.Hash
}
