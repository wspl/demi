package tabs

import (
	"context"
	"fmt"
	"log/slog"
	"path/filepath"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandproto"

	"github.com/wspl/demi/internal/commandsdk"
)

// ChromeName is the artifact line of the pinned release.
const ChromeName = "Chrome for Testing"

// Chrome accesses the runner's shared installation source. Its zero value is ready.
// It never discovers a system browser or downloads an alternative release.
type Chrome struct {
	// mu protects the replaceable artifact source and its notification.
	mu        sync.Mutex
	artifacts *commandsdk.Artifacts
	changed   chan struct{}
}

// Attach supplies the service's runner-backed artifacts source.
func (c *Chrome) Attach(artifacts *commandsdk.Artifacts) {
	c.mu.Lock()
	old := c.changed
	c.artifacts = artifacts
	c.changed = make(chan struct{})
	c.mu.Unlock()
	if old != nil {
		close(old)
	}
}

// Executable requests the pinned chrome resource for this invocation and Host.
func (c *Chrome) Executable(ctx context.Context, invocation string) (string, error) {
	release, err := browserproto.PinnedRelease()
	if err != nil {
		return "", &cdp.BrowserError{Kind: cdp.KindConfiguration, Cause: err, Message: err.Error()}
	}
	host, err := commandproto.HostTarget()
	if err != nil {
		return "", err
	}
	platform, ok := release.Platform(string(host))
	if !ok {
		return "", &cdp.BrowserError{
			Kind:    cdp.KindInstallation,
			Message: fmt.Sprintf("%s is unavailable on %s", release.Title(), host),
		}
	}
	c.mu.Lock()
	source := c.artifacts
	c.mu.Unlock()
	if source == nil {
		return "", &cdp.BrowserError{Kind: cdp.KindInstallation, Message: "demi-browser has no artifacts source"}
	}
	path, err := source.Install(
		ctx,
		commandproto.ArtifactInstall{
			Invocation: invocation,
			Name:       ChromeName,
			Version:    release.Version,
			SHA256:     platform.SHA256,
			Size:       platform.Size,
			Form:       &commandproto.ArtifactArchive{Entry: platform.Executable},
		},
	)
	if err != nil {
		return "", &cdp.BrowserError{Kind: cdp.KindInstallation, Message: err.Error(), Cause: err}
	}
	return path, nil
}

// Roots waits for the artifacts source and lists Chrome installation roots.
// Listing failures are logged and yield an empty list; context
// cancellation is returned so service shutdown can join the orphan sweep.
func (c *Chrome) Roots(ctx context.Context) ([]string, error) {
	for {
		c.mu.Lock()
		if c.changed == nil {
			c.changed = make(chan struct{})
		}
		source, changed := c.artifacts, c.changed
		c.mu.Unlock()
		if source != nil {
			installed, err := source.Installed(ctx, ChromeName)
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			if err != nil {
				slog.Warn("the installed Chrome could not be listed", "error", err)
				return nil, nil
			}
			roots := make([]string, 0, len(installed))
			for _, artifact := range installed {
				roots = append(roots, installationOf(artifact.Path))
			}
			return roots, nil
		}
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-changed:
		}
	}
}

// PinnedVersion returns the release's complete Chrome version.
func PinnedVersion() (string, error) {
	release, err := browserproto.PinnedRelease()
	if err != nil {
		return "", &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: err.Error(), Cause: err}
	}
	return release.Version, nil
}

// SweepOrphans removes only this user's unlocked environments and orphan profiles.
// Per-environment failures are logged and retained rather than aborting the sweep.
func SweepOrphans(ctx context.Context, chrome *Chrome) error {
	roots, err := chrome.Roots(ctx)
	if err != nil {
		return err
	}
	return sweepOrphans(ctx, HostDirectories(), roots, currentOwner())
}

// installationOf finds the root containing Chrome and its executable helpers.
func installationOf(executable string) string {
	// Process tables report physical executable paths (not /tmp's Darwin alias).
	if physical, err := filepath.EvalSymlinks(executable); err == nil {
		executable = physical
	}
	for path := executable; path != filepath.Dir(path); path = filepath.Dir(path) {
		if strings.HasSuffix(path, ".app") {
			return path
		}
	}
	return filepath.Dir(executable)
}
