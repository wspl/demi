package backend

import (
	"errors"
	"slices"
	"strings"

	"github.com/wspl/demi/go/core"
)

// ExposeDomain is the domain expose hostnames live under (expose.md §
// Deployment), such as expose.demi.example: a DNS name, in lowercase.
type ExposeDomain struct{ name string }

var errNotExposeDomain = errors.New("must be a domain name, such as expose.demi.example")

// ParseExposeDomain reads text as a URL's host is read: an internationalized
// name becomes its ASCII form, in lowercase. An IP address, a name with an
// empty label, and anything but a host are refused.
func ParseExposeDomain(text string) (ExposeDomain, error) {
	// The characters that would end a URL's host are no host's: without
	// this, "a.example/b" would read as a host and a path.
	if text == "" || strings.ContainsAny(text, "/?#@:\\") {
		return ExposeDomain{}, errNotExposeDomain
	}
	address, err := core.ParseURL("http://" + text + "/")
	if err != nil || address.IsIPv4() || address.IsIPv6() {
		return ExposeDomain{}, errNotExposeDomain
	}
	name := address.Hostname()
	if slices.Contains(strings.Split(name, "."), "") {
		return ExposeDomain{}, errNotExposeDomain
	}
	return ExposeDomain{name}, nil
}

// String is the domain, in lowercase.
func (d ExposeDomain) String() string { return d.name }

// Label is the label an expose hostname puts before the domain: host, a
// Host header's host without its port, is <label>.<domain> in any case, and
// the label has no dot. False for any other host.
func (d ExposeDomain) Label(host string) (string, bool) {
	prefix, found := strings.CutSuffix(strings.ToLower(host), "."+d.name)
	if !found || prefix == "" || strings.Contains(prefix, ".") {
		return "", false
	}
	return prefix, true
}
