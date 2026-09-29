package backend

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
	"unicode"

	"github.com/wspl/demi/go/claudeproto"
	"github.com/wspl/demi/go/gates"
)

// DefaultClaudeReleasesURL is the vendor's official distribution of Claude
// Code.
const DefaultClaudeReleasesURL = "https://downloads.claude.ai/claude-code-releases"

const (
	// claudeLatestTTL is how long the newest version is believed before the
	// pointer is read again.
	claudeLatestTTL = 6 * time.Hour
	// claudeReadTimeout is how long one read of the distribution may take.
	claudeReadTimeout = 15 * time.Second
)

// ReleaseError is why the newest release could not be read, in words that
// complete "Claude Code could not be installed: …".
type ReleaseError struct{ Reason string }

func (e *ReleaseError) Error() string { return e.Reason }

// claudeManifest is a version's manifest.json: each platform's executable
// name, SHA-256 and byte size. Members Demi does not read are ignored.
//
//demi:wire open
type claudeManifest struct {
	Version   string                            `json:"version"`
	Platforms map[string]claudeManifestPlatform `json:"platforms"`
}

//demi:wire open
type claudeManifestPlatform struct {
	Binary   string `json:"binary"`
	Checksum string `json:"checksum"`
	Size     uint64 `json:"size"`
}

// ClaudeReleases are the vendor's Claude Code releases (claude-code.md §
// Which version): the newest version, read from the distribution's latest
// pointer and that version's manifest, and believed for six hours.
// Concurrent readers share one read, a version's manifest is read once, and
// a redirect is refused.
type ClaudeReleases struct {
	// base is the distribution's address, without a trailing slash.
	base string
	// http follows no redirect and gives up after claudeReadTimeout.
	http *http.Client
	// turn is held across a read, so that readers who wait for it share
	// it; it guards the fields below.
	turn *gates.SerialGate
	// newest is the newest release and when it was read; nil before the
	// first read.
	newest *knownRelease
	// releases are each version's release, which never changes once
	// published.
	releases map[string]claudeproto.Release
}

type knownRelease struct {
	readAt  time.Time
	release claudeproto.Release
}

// NewClaudeReleases reads the releases of the distribution at base.
func NewClaudeReleases(base url.URL) *ClaudeReleases {
	client := &http.Client{
		Timeout: claudeReadTimeout,
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
	return &ClaudeReleases{base: strings.TrimRight(base.String(), "/"), http: client, turn: gates.NewSerialGate(), releases: map[string]claudeproto.Release{}}
}

// Latest is the vendor's newest release: the one read within six hours, or
// a new read. refresh reads the pointer again unless a read finished after
// the call began.
func (r *ClaudeReleases) Latest(ctx context.Context, refresh bool) (claudeproto.Release, error) {
	asked := time.Now()
	permit, err := r.turn.Acquire(ctx)
	if err != nil {
		return claudeproto.Release{}, err
	}
	defer permit.Release()
	if newest := r.newest; newest != nil {
		fresh := time.Since(newest.readAt) < claudeLatestTTL
		if refresh {
			fresh = !newest.readAt.Before(asked)
		}
		if fresh {
			return newest.release, nil
		}
	}
	pointer, err := r.text(ctx, "latest")
	if err != nil {
		return claudeproto.Release{}, &ReleaseError{err.Error()}
	}
	version := strings.TrimSpace(pointer)
	if !claudeproto.IsVersion(version) {
		return claudeproto.Release{}, &ReleaseError{"the distribution named no version"}
	}
	release, known := r.releases[version]
	if !known {
		release, err = r.release(ctx, version)
		if err != nil {
			return claudeproto.Release{}, &ReleaseError{fmt.Sprintf("the release of version %s could not be read (%s)", version, err)}
		}
		r.releases[version] = release
	}
	r.newest = &knownRelease{readAt: time.Now(), release: release}
	return release, nil
}

// release is version's release record, from its manifest.
func (r *ClaudeReleases) release(ctx context.Context, version string) (claudeproto.Release, error) {
	text, err := r.text(ctx, version+"/manifest.json")
	if err != nil {
		return claudeproto.Release{}, err
	}
	manifest, err := decode[claudeManifest]([]byte(text))
	if err != nil {
		return claudeproto.Release{}, err
	}
	if manifest.Version != version {
		return claudeproto.Release{}, fmt.Errorf("its manifest names version %s", manifest.Version)
	}
	release := claudeproto.Release{Version: manifest.Version, Platforms: map[string]claudeproto.Artifact{}}
	for platform, entry := range manifest.Platforms {
		release.Platforms[platform] = claudeproto.Artifact{
			URL:    fmt.Sprintf("%s/%s/%s/%s", r.base, version, platform, entry.Binary),
			Size:   entry.Size,
			SHA256: entry.Checksum,
		}
	}
	if err := claudeproto.Validate(release); err != nil {
		return claudeproto.Release{}, errors.New(strings.TrimRightFunc(err.Error(), unicode.IsSpace))
	}
	return release, nil
}

// text is the text at path under the distribution, or why it was not read.
func (r *ClaudeReleases) text(ctx context.Context, path string) (string, error) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, r.base+"/"+path, nil)
	if err != nil {
		return "", fmt.Errorf("the distribution did not answer (%w)", err)
	}
	response, err := r.http.Do(request)
	if err != nil {
		return "", fmt.Errorf("the distribution did not answer (%w)", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode > 299 {
		return "", fmt.Errorf("the distribution answered %d %s", response.StatusCode, http.StatusText(response.StatusCode))
	}
	body, err := io.ReadAll(response.Body)
	if err != nil {
		return "", fmt.Errorf("the distribution did not answer (%w)", err)
	}
	return string(body), nil
}
