package edge

import (
	"net/url"
	"sync/atomic"
)

// Site describes how the backend is reached from outside, and what it serves
// besides the API. Its zero value is ready. Configure it before Start, then
// share its pointer without copying or changing its configuration.
type Site struct {
	// PublicURL is the URL the product's pages and the runners reach the backend
	// at; nil means a request's own origin.
	PublicURL *url.URL
	// RunnerReleases is the directory of runner releases the installers serve:
	// manifest.json names the current one, and each release's directory holds
	// its own and one executable per target. Empty means not configured.
	RunnerReleases string
	// OriginDropped records whether a request showed that a proxy in front of
	// the backend drops Origin, which the edge warns about once.
	OriginDropped atomic.Bool
}
