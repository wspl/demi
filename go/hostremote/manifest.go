// Package hostremote implements the backend end of the runner protocol. The
// embedding backend owns routes, sockets, Host admission and product policy.
package hostremote

import (
	"context"
	"slices"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

// ArtifactResolver supplies an executable location while its job or service
// lives. Cancellation releases all resolver work for that owner.
type ArtifactResolver interface {
	Resolve(context.Context, commandservice.PackageArtifact, string) (commandservice.ArtifactLocation, error)
}
type CommandCatalog struct {
	packages []commandservice.PackageDescriptor
	resolver ArtifactResolver
}

func NewCommandCatalog(packages []commandservice.PackageDescriptor, resolver ArtifactResolver) (*CommandCatalog, error) {
	if _, err := runnerproto.BuildManifest(nil, packages); err != nil {
		return nil, err
	}
	return &CommandCatalog{slices.Clone(packages), resolver}, nil
}
func (c *CommandCatalog) Select(commands *shell.CommandSet) (*CommandSelection, error) {
	manifest, err := runnerproto.BuildManifest(commands.Declarations(), c.packages)
	if err != nil {
		return nil, err
	}
	wire, err := manifest.Encode()
	if err != nil {
		return nil, err
	}
	return &CommandSelection{manifest, wire, c.resolver}, nil
}

// CommandSelection is immutable once published to a job.
type CommandSelection struct {
	manifest runnerproto.Manifest
	wire     []byte
	resolver ArtifactResolver
}

func (s *CommandSelection) Manifest() runnerproto.Manifest { return s.manifest }
func (s *CommandSelection) Hash() string                   { return s.manifest.Hash }
