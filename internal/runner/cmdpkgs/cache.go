//revive:disable:unused-parameter API checkpoint retains parameter names for callers; bodies follow after merge.

package cmdpkgs

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
)

// ArtifactCache owns verified files, unpacked archives, line records and image copies.
// Construct it with NewArtifactCache.
type ArtifactCache struct{}

// Wanted identifies an artifact's line, version, exact bytes and installed form.
type Wanted struct {
	Package  string
	Name     string
	Version  string
	Artifact commandwire.PackageArtifact
	Form     commandwire.ArtifactForm
}

// NewArtifactCache opens root and reports downloads through installs.
// An empty image means this Host has no preinstalled image artifacts.
func NewArtifactCache(ctx context.Context, root, image string, installs *Installs) (*ArtifactCache, error) {
	panic("not written: r-cmdpkgs")
}

// Close cancels and joins cache-owned installation work and releases HTTP resources.
// If ctx ends first, a later Close can wait for shutdown. Close is idempotent.
func (c *ArtifactCache) Close(ctx context.Context) error { panic("not written: r-cmdpkgs") }

// Holds returns the digests running services hold against line removal.
func (c *ArtifactCache) Holds() *Holds { panic("not written: r-cmdpkgs") }

// Install returns the cached, verified image, or downloaded path of wanted.
// It records the line and removes older artifacts no service holds. Metadata
// damage in an existing entry fails the install.
func (c *ArtifactCache) Install(ctx context.Context, wanted Wanted, resolver ArtifactResolver) (string, error) {
	panic("not written: r-cmdpkgs")
}

// Installed lists artifacts installed from downloads or the image, newest first.
func (c *ArtifactCache) Installed(ctx context.Context, pkg, name string) ([]commandwire.InstalledArtifact, error) {
	panic("not written: r-cmdpkgs")
}

// Holds counts digests protected from removal. Its zero value is ready for use.
// Do not copy it after first use.
type Holds struct{}

// Hold protects one digest until Release is called.
type Hold struct{}

// Hold claims sha256 until the returned hold is released.
func (h *Holds) Hold(sha256 string) *Hold { panic("not written: r-cmdpkgs") }

// Release ends this hold exactly once; repeated calls do nothing.
func (h *Hold) Release() { panic("not written: r-cmdpkgs") }
