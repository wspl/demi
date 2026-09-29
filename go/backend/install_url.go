package backend

import (
	"strings"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

// InstallableURL says whether address can be the backend URL an installer
// names (backend.md § Configuration): an http or https URL without a user, a
// password, a query or a fragment. An empty query or fragment counts: the
// parser keeps a bare "?" and "#" in the URL's text.
func InstallableURL(address *whatwg.Url) bool {
	scheme := address.Scheme()
	withoutFragment := address.Href(true)
	return (scheme == "http" || scheme == "https") &&
		address.Username() == "" &&
		address.Password() == "" &&
		!strings.Contains(withoutFragment, "?") &&
		withoutFragment == address.Href(false)
}
