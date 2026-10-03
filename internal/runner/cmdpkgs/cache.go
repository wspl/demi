package cmdpkgs

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"sync"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/process"
)

// ArtifactCache owns verified files, unpacked archives, line records and image copies.
// Construct it with NewArtifactCache.
type ArtifactCache struct {
	root, image string
	http        *artifacts.Client
	installs    *Installs
	holds       Holds
	mu          sync.Mutex
	checked     map[string]string
	active      map[string]*installation
	closed      bool
	work        sync.WaitGroup
	done        chan struct{}
}

type installation struct {
	ctx     context.Context
	cancel  context.CancelFunc
	done    chan struct{}
	waiters int
	path    string
	err     error
}

// Wanted identifies an artifact's line, version, exact bytes and installed form.
type Wanted struct {
	// Package identifies the package that owns the artifact.
	Package string
	// Name identifies the resource within the package.
	Name string
	// Version is the package version.
	Version string
	// Artifact describes the expected artifact bytes.
	Artifact commandwire.PackageArtifact
	// Form selects a file or extracted archive.
	Form commandwire.ArtifactForm
}

// NewArtifactCache opens root and reports downloads through installs.
// An empty image means this Host has no preinstalled image artifacts.
func NewArtifactCache(ctx context.Context, root, image string, installs *Installs) (*ArtifactCache, error) {
	if err := ctx.Err(); err != nil {
		return nil, runtimeFailure(err)
	}
	if err := os.MkdirAll(root, 0o700); err != nil {
		return nil, runtimeFailure(err)
	}
	if err := process.Chmod(ctx, root, 0o700); err != nil {
		return nil, runtimeFailure(err)
	}
	return &ArtifactCache{
		root:     root,
		image:    image,
		installs: installs,
		http:     artifacts.NewClientAllowingHTTP(),
		checked:  make(map[string]string),
		active:   make(map[string]*installation),
		done:     make(chan struct{}),
	}, nil
}

// Close cancels and joins cache-owned installation work and releases HTTP resources.
// If ctx ends first, a later Close can wait for shutdown. Close is idempotent.
func (c *ArtifactCache) Close(ctx context.Context) error {
	c.mu.Lock()
	first := !c.closed
	c.closed = true
	pending := make([]*installation, 0, len(c.active))
	if first {
		for _, job := range c.active {
			pending = append(pending, job)
		}
	}
	c.mu.Unlock()
	if first {
		for _, job := range pending {
			job.cancel()
		}
		go func() {
			c.work.Wait()
			c.http.Close()
			close(c.done)
		}()
	}
	select {
	case <-c.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Holds returns the digests running services hold against line removal.
func (c *ArtifactCache) Holds() *Holds { return &c.holds }

// Install returns the cached, verified image, or downloaded path of wanted.
// It records the line and removes older artifacts no service holds. Metadata
// damage in an existing entry fails the install.
func (c *ArtifactCache) Install(ctx context.Context, wanted Wanted, resolver ArtifactResolver) (string, error) {
	if err := ctx.Err(); err != nil {
		return "", &RuntimeError{Kind: Cancelled, Cause: err}
	}
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return "", &RuntimeError{Kind: Stopped}
	}
	job := c.active[wanted.Artifact.SHA256]
	if job == nil {
		workctx, cancel := context.WithCancel(context.WithoutCancel(ctx))
		job = &installation{ctx: workctx, cancel: cancel, done: make(chan struct{})}
		c.active[wanted.Artifact.SHA256] = job
		c.work.Add(1)
		go c.install(job, wanted, resolver)
	}
	job.waiters++
	c.mu.Unlock()
	defer func() {
		c.mu.Lock()
		job.waiters--
		abandoned := job.waiters == 0
		if abandoned && c.active[wanted.Artifact.SHA256] == job {
			delete(c.active, wanted.Artifact.SHA256)
		}
		c.mu.Unlock()
		if abandoned {
			job.cancel()
		}
	}()
	select {
	case <-ctx.Done():
		return "", &RuntimeError{Kind: Cancelled, Cause: ctx.Err()}
	case <-job.done:
		return job.path, job.err
	}
}

func (c *ArtifactCache) install(job *installation, wanted Wanted, resolver ArtifactResolver) {
	defer c.work.Done()
	defer job.cancel()
	path, err := cmdsdk.Retry(job.ctx, func() (string, error) {
		if err := job.ctx.Err(); err != nil {
			return "", &RuntimeError{Kind: Cancelled, Cause: err}
		}
		return c.obtain(job.ctx, wanted, resolver)
	})
	if err == nil {
		err = c.record(job.ctx, wanted, path)
	}
	if err == nil {
		c.removeOlder(job.ctx, wanted)
	}
	c.mu.Lock()
	job.path = path
	job.err = runtimeFailure(err)
	if c.active[wanted.Artifact.SHA256] == job {
		delete(c.active, wanted.Artifact.SHA256)
	}
	close(job.done)
	c.mu.Unlock()
}

func (c *ArtifactCache) obtain(ctx context.Context, wanted Wanted, resolver ArtifactResolver) (path string, err error) {
	destination := filepath.Join(c.root, wanted.Artifact.SHA256)
	switch form := wanted.Form.(type) {
	case *commandwire.ArtifactFile:
		return c.obtainFile(ctx, wanted, resolver, destination)
	case *commandwire.ArtifactArchive:
		return c.obtainArchive(ctx, wanted, resolver, destination, form)
	}
	return "", &RuntimeError{Kind: ArtifactFailure, Cause: os.ErrInvalid}
}

func digestOf(a commandwire.PackageArtifact) artifacts.Digest {
	return artifacts.Digest{Size: a.Size, SHA256: a.SHA256}
}

func artifactError(err error) error {
	if err == nil {
		return nil
	}
	return &RuntimeError{Kind: ArtifactFailure, Cause: err}
}

func cached(path string, artifact commandwire.PackageArtifact) (string, error) {
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	if !info.Mode().IsRegular() {
		return "", artifactError(artifacts.ErrDigest)
	}
	if uint64(info.Size()) != artifact.Size {
		return "", artifactError(&artifacts.SizeError{Declared: artifact.Size, Actual: uint64(info.Size())})
	}
	return path, nil
}

// Holds counts digests protected from removal. Its zero value is ready for use.
// Do not copy it after first use.
type Holds struct {
	mu     sync.Mutex
	counts map[string]int
}

// Hold protects one digest until Release is called.
type Hold struct{ release func() }

// Hold claims sha256 until the returned hold is released.
func (h *Holds) Hold(sha256 string) *Hold {
	h.mu.Lock()
	if h.counts == nil {
		h.counts = make(map[string]int)
	}
	h.counts[sha256]++
	h.mu.Unlock()
	return &Hold{release: sync.OnceFunc(func() {
		h.mu.Lock()
		defer h.mu.Unlock()
		h.counts[sha256]--
		if h.counts[sha256] == 0 {
			delete(h.counts, sha256)
		}
	})}
}

func (h *Holds) held(sha256 string) bool {
	h.mu.Lock()
	defer h.mu.Unlock()
	return h.counts[sha256] > 0
}

// Release ends this hold exactly once; repeated calls do nothing.
func (h *Hold) Release() { h.release() }

func (c *ArtifactCache) obtainFile(
	ctx context.Context,
	wanted Wanted,
	resolver ArtifactResolver,
	destination string,
) (path string, err error) {
	if path, err = cached(destination, wanted.Artifact); path != "" || err != nil {
		return path, err
	}
	if path, err = c.preinstalled(ctx, wanted); path != "" || err != nil {
		return path, err
	}
	installing := c.installs.start(wanted)
	defer installing.close()
	var staged *artifacts.Staged
	staged, err = artifacts.NewStaged(
		ctx,
		destination,
		artifacts.Publication{Mode: artifacts.CreateNew, Permissions: artifacts.Executable, Durable: true},
	)
	if err != nil {
		return "", artifactError(err)
	}
	defer func() { err = errors.Join(err, staged.Close()) }()
	if err := c.fetch(ctx, wanted.Artifact, staged.File(), resolver, installing); err != nil {
		return "", err
	}
	if err := staged.Publish(ctx); err != nil {
		if errors.Is(err, os.ErrExist) {
			return cached(destination, wanted.Artifact)
		}
		return "", artifactError(err)
	}
	return destination, nil
}

func (c *ArtifactCache) obtainArchive(
	ctx context.Context,
	wanted Wanted,
	resolver ArtifactResolver,
	destination string,
	form *commandwire.ArtifactArchive,
) (path string, err error) {
	archive := artifacts.Archive{Digest: digestOf(wanted.Artifact), Entry: form.Entry}
	path, err = artifacts.Recorded(ctx, destination, archive)
	if path != "" || err != nil {
		return path, artifactError(err)
	}
	if path, err = c.preinstalled(ctx, wanted); path != "" || err != nil {
		return path, err
	}
	var unpacking *artifacts.Unpacking
	path, unpacking, err = artifacts.InstallArchive(ctx, c.root, archive)
	if err != nil {
		return "", artifactError(err)
	}
	if unpacking == nil {
		return path, nil
	}
	defer func() { err = errors.Join(err, unpacking.Close()) }()
	installing := c.installs.start(wanted)
	defer installing.close()
	var output *os.File
	output, err = os.Create(unpacking.ArchivePath())
	if err != nil {
		return "", err
	}
	fetchErr := c.fetch(ctx, wanted.Artifact, output, resolver, installing)
	if err := errors.Join(fetchErr, output.Close()); err != nil {
		return "", err
	}
	installing.unpacking()
	path, err = unpacking.Finish(ctx)
	return path, artifactError(err)
}
