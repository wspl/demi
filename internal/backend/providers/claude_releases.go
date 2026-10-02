package providers

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
)

// DefaultReleasesURL is the vendor's official distribution.
const DefaultReleasesURL = "https://downloads.claude.ai/claude-code-releases"

// ClaudeReleases owns shared reads of the vendor's releases.
type ClaudeReleases struct {
	base string
	http *http.Client
	// mu protects cached releases and refresh admission, never network reads.
	mu       sync.Mutex
	releases map[string]claudecodeop.Release
	newest   *claudecodeop.Release
	readAt   time.Time
	pending  *releaseRead
	ctx      context.Context
	cancel   context.CancelFunc
	workers  sync.WaitGroup
}
type releaseRead struct {
	done    chan struct{}
	release claudecodeop.Release
	err     error
}
type releaseManifest struct {
	Version   string                     `json:"version"`
	Platforms map[string]releasePlatform `json:"platforms"`
}
type releasePlatform struct {
	Binary   string `json:"binary"`
	Checksum string `json:"checksum"`
	Size     uint64 `json:"size"`
}

// CLIPackage returns the command package of a process provider, or nil.
func CLIPackage(p provider.Provider) *string {
	if p.Capabilities().ProcessHost {
		name := claudecodeop.Package
		return &name
	}
	return nil
}

// NewClaudeReleases creates a distribution reader that refuses redirects.
func NewClaudeReleases(base *url.URL) (*ClaudeReleases, error) {
	ctx, cancel := context.WithCancel(context.Background())
	return &ClaudeReleases{base: strings.TrimRight(base.String(), "/"), http: &http.Client{Timeout: 15 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, releases: make(map[string]claudecodeop.Release), ctx: ctx, cancel: cancel}, nil
}

// Latest returns the newest release, believed for six hours unless refreshed.
func (r *ClaudeReleases) Latest(ctx context.Context, refresh bool) (claudecodeop.Release, error) {
	asked := time.Now()
	r.mu.Lock()
	if r.ctx.Err() != nil {
		r.mu.Unlock()
		return claudecodeop.Release{}, r.ctx.Err()
	}
	if r.newest != nil && ((!refresh && time.Since(r.readAt) < 6*time.Hour) || (refresh && !r.readAt.Before(asked))) {
		result := *r.newest
		r.mu.Unlock()
		return copyRelease(result)
	}
	pending := r.pending
	if pending == nil {
		pending = &releaseRead{done: make(chan struct{})}
		r.pending = pending
		r.workers.Go(func() { r.read(pending) })
	}
	r.mu.Unlock()
	select {
	case <-ctx.Done():
		return claudecodeop.Release{}, ctx.Err()
	case <-pending.done:
	}
	if pending.err != nil {
		return claudecodeop.Release{}, pending.err
	}
	return copyRelease(pending.release)
}

// Close cancels and joins distribution reads.
func (r *ClaudeReleases) Close(_ context.Context) error {
	r.mu.Lock()
	r.cancel()
	r.mu.Unlock()
	r.workers.Wait()
	r.http.CloseIdleConnections()
	return nil
}

func (r *ClaudeReleases) text(ctx context.Context, path string) (string, error) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, r.base+"/"+path, nil)
	if err != nil {
		return "", fmt.Errorf("the distribution did not answer (%w)", err)
	}
	response, err := r.http.Do(request)
	if err != nil {
		return "", fmt.Errorf("the distribution did not answer (%w)", err)
	}
	// The response body is read to completion or discarded; a close error cannot change the read outcome.
	defer func() { _ = response.Body.Close() }()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return "", fmt.Errorf("the distribution answered %s", response.Status)
	}
	data, err := io.ReadAll(response.Body)
	if err != nil {
		return "", fmt.Errorf("the distribution did not answer (%w)", err)
	}
	return string(data), nil
}
func (r *ClaudeReleases) read(pending *releaseRead) {
	release, err := r.fetch()
	r.mu.Lock()
	if err == nil {
		r.newest = &release
		r.readAt = time.Now()
	}
	pending.release = release
	if err != nil {
		pending.err = &ReleaseError{Message: err.Error()}
	}
	r.pending = nil
	close(pending.done)
	r.mu.Unlock()
}
func (r *ClaudeReleases) fetch() (claudecodeop.Release, error) {
	pointer, err := r.text(r.ctx, "latest")
	if err != nil {
		return claudecodeop.Release{}, err
	}
	version := strings.TrimSpace(pointer)
	if err := claudecodeop.Version(version).Validate(); err != nil {
		return claudecodeop.Release{}, fmt.Errorf("the distribution named no version")
	}
	r.mu.Lock()
	release, ok := r.releases[version]
	r.mu.Unlock()
	if ok {
		return release, nil
	}
	release, err = r.manifest(version)
	if err != nil {
		return claudecodeop.Release{}, fmt.Errorf("the release of version %s could not be read (%w)", version, err)
	}
	r.mu.Lock()
	r.releases[version] = release
	r.mu.Unlock()
	return release, nil
}
func (r *ClaudeReleases) manifest(version string) (claudecodeop.Release, error) {
	text, err := r.text(r.ctx, version+"/manifest.json")
	if err != nil {
		return claudecodeop.Release{}, err
	}
	manifest, err := provider.DecodeUntagged[releaseManifest](text)
	if err != nil {
		return claudecodeop.Release{}, err
	}
	if manifest.Version != version {
		return claudecodeop.Release{}, fmt.Errorf("its manifest names version %s", manifest.Version)
	}
	release := claudecodeop.Release{Version: claudecodeop.Version(version), Platforms: make(map[string]claudecodeop.Artifact)}
	for platform, entry := range manifest.Platforms {
		release.Platforms[platform] = claudecodeop.Artifact{URL: r.base + "/" + version + "/" + platform + "/" + entry.Binary, Size: entry.Size, SHA256: entry.Checksum}
	}
	if err := release.Validate(); err != nil {
		return claudecodeop.Release{}, err
	}
	return release, nil
}

// copyRelease keeps callers from mutating the distribution's cached manifest.
func copyRelease(release claudecodeop.Release) (claudecodeop.Release, error) {
	data, err := contract.EncodeJSON(release)
	if err != nil {
		return claudecodeop.Release{}, err
	}
	return claudecodeop.DecodeRelease(data)
}
