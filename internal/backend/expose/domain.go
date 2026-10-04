package expose

import (
	"errors"
	"strings"

	"github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/webapiproto"
)

// Domain is the domain, such as expose.demi.example: a DNS name, in lowercase.
type Domain struct{ name string }

// ErrNotDomain means the text is not an expose domain.
var ErrNotDomain = errors.New("must be a domain name, such as expose.demi.example")

// ParseDomain parses a domain using the same special-host rules as public URLs.
func ParseDomain(text string) (Domain, error) {
	// A host parser must not interpret a delimiter as another URL component,
	// nor let the URL parser strip whitespace, which a domain never contains.
	if text == "" || strings.ContainsAny(text, "\x00\t\n\r #/:<>?@[\\]^|") {
		return Domain{}, ErrNotDomain
	}
	parsed, err := url.Parse("http://" + text + "/")
	if err != nil || parsed.IsIPv4() || parsed.IsIPv6() {
		return Domain{}, ErrNotDomain
	}
	domain := parsed.Hostname()
	for _, label := range strings.Split(domain, ".") {
		if label == "" {
			return Domain{}, ErrNotDomain
		}
	}
	return Domain{name: domain}, nil
}

// String returns the domain in lowercase.
func (d Domain) String() string {
	return d.name
}

// Label returns the single label before this domain, ignoring ASCII case.
// The caller removes the Host header's port first.
func (d Domain) Label(host string) (string, bool) {
	host = strings.Map(func(r rune) rune {
		if r >= 'A' && r <= 'Z' {
			return r + ('a' - 'A')
		}
		return r
	}, host)
	label, ok := strings.CutSuffix(host, "."+d.name)
	return label, ok && label != "" && !strings.Contains(label, ".")
}

// URL returns an expose's public URL with the backend's scheme and nondefault port.
func URL(id webapiproto.ExposeID, domain Domain, backend *url.Url) string {
	port := backend.Port()
	if port != "" {
		port = ":" + port
	}
	return backend.Scheme() + "://" + string(id) + "." + domain.String() + port + "/"
}
