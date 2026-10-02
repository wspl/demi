package remotehost

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// ArtifactResolver locates a package executable while the requesting work lives.
type ArtifactResolver interface {
	Resolve(context.Context, commandwire.PackageArtifact, string) (commandwire.ArtifactLocation, error)
}

// CommandCatalog binds commands to package descriptors and executable locations.
type CommandCatalog struct {
	packages []commandwire.PackageDescriptor
	resolver ArtifactResolver
}

// NewCommandCatalog validates descriptors and requires unique package IDs.
func NewCommandCatalog(packages []commandwire.PackageDescriptor, resolver ArtifactResolver) (*CommandCatalog, error) {
	manifest, err := runnerwire.BuildManifest(nil, packages)
	if err != nil {
		return nil, err
	}
	data, err := manifest.MarshalJSON()
	if err != nil {
		return nil, err
	}
	owned, err := runnerwire.DecodeManifest(data)
	if err != nil {
		return nil, err
	}
	descriptors := make([]commandwire.PackageDescriptor, 0, len(owned.Packages))
	for _, descriptor := range owned.Packages {
		descriptors = append(descriptors, descriptor)
	}
	return &CommandCatalog{packages: descriptors, resolver: resolver}, nil
}

// Select pins each native command to its descriptor in the catalog.
func (c *CommandCatalog) Select(commands *host.CommandSet) (*CommandSelection, error) {
	manifest, err := runnerwire.BuildManifest(commands.Declarations(), c.packages)
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
	manifest runnerwire.Manifest
	wire     []byte
	resolver ArtifactResolver
}

// Manifest returns the pinned manifest, which callers must treat as immutable.
func (s *CommandSelection) Manifest() runnerwire.Manifest {
	return s.manifest
}

// Hash returns the pinned manifest's hash.
func (s *CommandSelection) Hash() string {
	return s.manifest.Hash
}
