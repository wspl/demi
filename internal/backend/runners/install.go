package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"net/url"

	"github.com/wspl/demi/internal/runnerwire"
)

// InvalidBackendURL identifies a URL that an installer cannot name: not HTTP(S),
// or containing credentials, a query or a fragment.
type InvalidBackendURL struct {
	// URL is the rejected installation backend URL.
	URL string
}

// Error describes why the URL cannot name an installation's backend.
func (e *InvalidBackendURL) Error() string { panic("not written: b-runners") }

// BackendURL checks and returns the URL as an installer names it.
func BackendURL(value *url.URL) (*url.URL, error) { panic("not written: b-runners") }

// ShellScript renders the macOS and Linux shell installer for backend and release.
func ShellScript(backend *url.URL, release runnerwire.RunnerRelease) string {
	panic("not written: b-runners")
}

// PowerShellScript renders the installer for the release's Windows targets.
func PowerShellScript(backend *url.URL, release runnerwire.RunnerRelease) string {
	panic("not written: b-runners")
}
