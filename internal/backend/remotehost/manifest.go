package remotehost

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

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
type CommandCatalog struct{ _ byte }

// NewCommandCatalog validates descriptors and requires unique package IDs.
func NewCommandCatalog(packages []commandwire.PackageDescriptor, resolver ArtifactResolver) (*CommandCatalog, error) {
	panic("not written: b-remotehost")
}

// Select pins each native command to its descriptor in the catalog.
func (c *CommandCatalog) Select(commands *host.CommandSet) (*CommandSelection, error) {
	panic("not written: b-remotehost")
}

// CommandSelection is a job's pinned manifest and artifact resolver.
type CommandSelection struct{ _ byte }

// Manifest returns the pinned manifest, which callers must treat as immutable.
func (s *CommandSelection) Manifest() runnerwire.Manifest { panic("not written: b-remotehost") }

// Hash returns the pinned manifest's hash.
func (s *CommandSelection) Hash() string { panic("not written: b-remotehost") }
