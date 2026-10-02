package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/wspl/demi/internal/cmdsdk"
)

// ChromeName is the artifact line of the pinned release.
const ChromeName = "Chrome for Testing"

// Chrome accesses the runner's shared installation source. Its zero value is ready.
// It never discovers a system browser or downloads an alternative release.
type Chrome struct{}

// Attach supplies the service's runner-backed artifacts source.
func (c *Chrome) Attach(artifacts *cmdsdk.Artifacts) { panic("not written: k-chrome-tabs") }

// Executable requests the pinned chrome resource for this invocation and Host.
func (c *Chrome) Executable(ctx context.Context, invocation string) (string, error) {
	panic("not written: k-chrome-tabs")
}

// Roots waits for the artifacts source and lists Chrome installation roots.
// Listing failures are logged and yield an empty list, as in Rust; context
// cancellation is returned so service shutdown can join the orphan sweep.
func (c *Chrome) Roots(ctx context.Context) ([]string, error) { panic("not written: k-chrome-tabs") }

// PinnedVersion returns the release's complete Chrome version.
func PinnedVersion() (string, error) { panic("not written: k-chrome-tabs") }

// SweepOrphans removes only this user's unlocked environments and orphan profiles.
// Per-environment failures are logged and retained rather than aborting the sweep.
func SweepOrphans(ctx context.Context, chrome *Chrome) error { panic("not written: k-chrome-tabs") }
