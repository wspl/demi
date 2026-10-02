//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"context"
	"net/url"

	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/provider"
)

// DefaultReleasesURL is the vendor's official distribution.
const DefaultReleasesURL = "https://downloads.claude.ai/claude-code-releases"

// ClaudeReleases owns shared reads of the vendor's releases.
type ClaudeReleases struct{}

// CLIPackage returns the command package of a process provider, or nil.
func CLIPackage(p provider.Provider) *string { panic("not written: b-providers") }

// NewClaudeReleases creates a distribution reader that refuses redirects.
func NewClaudeReleases(base *url.URL) (*ClaudeReleases, error) { panic("not written: b-providers") }

// Latest returns the newest release, believed for six hours unless refreshed.
func (r *ClaudeReleases) Latest(ctx context.Context, refresh bool) (claudecodeop.Release, error) {
	panic("not written: b-providers")
}

// Close cancels and joins distribution reads.
func (r *ClaudeReleases) Close(ctx context.Context) error { panic("not written: b-providers") }
