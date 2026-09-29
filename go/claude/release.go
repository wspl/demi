package claude

import (
	"maps"
	"net/url"
	"slices"
	"strings"

	"github.com/wspl/demi/go/claudeproto"
)

// parseRelease decodes a release record and checks that every platform's URL may
// be downloaded, not only this machine's: HTTPS only, and plain HTTP to
// 127.0.0.1 when a test allows it. The record's shape is
// [claudeproto.Release].
func parseRelease(input []byte, allowLoopbackHTTP bool) (claudeproto.Release, error) {
	release, err := claudeproto.Decode[claudeproto.Release](input)
	if err != nil {
		return claudeproto.Release{}, invalidRelease(err.Error())
	}
	for _, platform := range slices.Sorted(maps.Keys(release.Platforms)) {
		if reason := allowed(release.Platforms[platform], allowLoopbackHTTP); reason != "" {
			return claudeproto.Release{}, invalidRelease("platform " + platform + ": " + reason)
		}
	}
	return release, nil
}

// allowed says why a platform's URL may not be downloaded, or is empty when it
// may.
func allowed(artifact claudeproto.Artifact, allowLoopbackHTTP bool) string {
	address, err := url.Parse(artifact.URL)
	if err != nil {
		return "url: is not a URL"
	}
	loopback := allowLoopbackHTTP && address.Scheme == "http" && address.Hostname() == "127.0.0.1"
	if address.Scheme != "https" && !loopback {
		return "url must be https"
	}
	return ""
}

// host returns the URL's host, for messages.
func host(artifact claudeproto.Artifact) string {
	address, err := url.Parse(artifact.URL)
	if err != nil {
		return ""
	}
	host := address.Hostname()
	if strings.Contains(host, ":") {
		// An IPv6 address is written in brackets.
		return "[" + host + "]"
	}
	return host
}
