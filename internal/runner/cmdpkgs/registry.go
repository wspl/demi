package cmdpkgs

import (
	"context"
	"errors"
	"log/slog"
	"maps"
	"sync"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// ServiceRegistry owns resident services and their lifecycle work. It must be closed.
type ServiceRegistry struct {
	mu          sync.Mutex
	cache       *ArtifactCache
	installs    Installs
	cwd         string
	env         map[string]string
	entries     map[string]*serviceEntry
	work        sync.WaitGroup
	closed      bool
	done        chan struct{}
	stop        context.Context
	cancel      context.CancelFunc
	invocations map[string]*Invoking
	decisions   decisionLog
}

type serviceEntry struct {
	leases  int
	changes uint64
	current *serviceLife
}

type serviceLife struct {
	service         string
	stop            context.Context
	cancel          context.CancelFunc
	ready           chan struct{}
	done            chan struct{}
	client          *cmdsdk.Client
	connectionEnded <-chan struct{}
	info            commandwire.ServiceInfo
	err             error
	checking        bool
}

// NewServiceRegistry starts services in cwd with exactly env, caching artifacts in
// cache and consulting image first when nonempty. The caller must Close the registry.
func NewServiceRegistry(
	ctx context.Context,
	cache, image, cwd string,
	env map[string]string,
) (*ServiceRegistry, error) {
	stop, cancel := context.WithCancel(context.WithoutCancel(ctx))
	r := &ServiceRegistry{
		cwd:         cwd,
		env:         maps.Clone(env),
		entries:     make(map[string]*serviceEntry),
		done:        make(chan struct{}),
		stop:        stop,
		cancel:      cancel,
		invocations: make(map[string]*Invoking),
	}
	cacheOwner, err := NewArtifactCache(ctx, cache, image, &r.installs)
	if err != nil {
		cancel()
		return nil, err
	}
	r.cache = cacheOwner
	return r, nil
}

// Handle returns a handle sharing this registry.
func (r *ServiceRegistry) Handle() *ServiceHandle { return &ServiceHandle{registry: r} }

// Installs observes the installs made by service starts and artifact requests.
func (r *ServiceRegistry) Installs() *InstallsReceiver { return r.installs.Subscribe() }

// Close stops every service and joins the registry's work. If ctx ends first,
// shutdown continues and a later Close can wait for it. Close is idempotent.
func (r *ServiceRegistry) Close(ctx context.Context) error {
	r.mu.Lock()
	first := !r.closed
	r.closed = true
	r.mu.Unlock()
	if first {
		r.cancel()
		go func() {
			r.work.Wait()
			// Background joins are owned by Close, including when its caller stops waiting.
			_ = r.cache.Close(context.Background())
			r.installs.Close()
			r.decisions.close()
			close(r.done)
		}()
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-r.done:
		return nil
	}
}

// ServiceHandle sends requests to one shared registry; it is safe for concurrent use.
type ServiceHandle struct{ registry *ServiceRegistry }

// Invoking registers invocation in pkg against its work's artifact resolver.
// The caller must release the returned registration when the invocation ends.
func (h *ServiceHandle) Invoking(invocation, pkg string, resolver ArtifactResolver) *Invoking {
	r := h.registry
	ctx, cancel := context.WithCancel(r.stop)
	i := &Invoking{registry: r, id: invocation, pkg: pkg, resolver: resolver, ctx: ctx, cancel: cancel}
	r.mu.Lock()
	r.invocations[invocation] = i
	r.mu.Unlock()
	return i
}

// Lease keeps digest's service resident until Release, even before it starts.
func (h *ServiceHandle) Lease(ctx context.Context, digest string) (*ServiceLease, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	r := h.registry
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.closed {
		return &ServiceLease{release: func() {}}, nil
	}
	return r.leaseLocked(digest), nil
}

// Acquire returns descriptor's service for this Host. Concurrent callers share
// one start, while cancellation abandons only this caller's wait.
func (h *ServiceHandle) Acquire(
	ctx context.Context,
	descriptor commandwire.PackageDescriptor,
	resolver ArtifactResolver,
	numbers NumberSource,
) (*Resident, error) {
	if err := ctx.Err(); err != nil {
		return nil, &RuntimeError{Kind: Cancelled, Cause: err}
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		return nil, &RuntimeError{Kind: CatalogMismatch}
	}
	artifact, ok := descriptor.Targets[string(target)]
	if !ok {
		return nil, &RuntimeError{Kind: CatalogMismatch}
	}
	r := h.registry
	r.mu.Lock()
	if r.closed {
		r.mu.Unlock()
		return nil, &RuntimeError{Kind: Cancelled}
	}
	waiting := r.leaseLocked(artifact.SHA256)
	entry := r.entries[artifact.SHA256]
	life := entry.current
	if life == nil {
		stop, cancel := context.WithCancel(r.stop)
		life = &serviceLife{
			service: descriptor.ID,
			stop:    stop,
			cancel:  cancel,
			ready:   make(chan struct{}),
			done:    make(chan struct{}),
		}
		entry.current = life
		r.work.Add(1)
		go r.live(life, descriptor, artifact, resolver, numbers)
	}
	r.mu.Unlock()
	defer waiting.Release()
	select {
	case <-ctx.Done():
		return nil, &RuntimeError{Kind: Cancelled, Cause: ctx.Err()}
	case <-life.ready:
		r.mu.Lock()
		defer r.mu.Unlock()
		if life.err != nil {
			return nil, life.err
		}
		if !descriptor.Serves(life.info) {
			return nil, &RuntimeError{Kind: CatalogMismatch}
		}
		return &Resident{life: life}, nil
	}
}

// ReleaseConversation releases a conversation in all running services concurrently
// and retires each service whose release fails.
func (h *ServiceHandle) ReleaseConversation(ctx context.Context, conversation string) error {
	r := h.registry
	r.mu.Lock()
	if r.closed {
		r.mu.Unlock()
		return errors.New("the service registry is closed")
	}
	running := make(map[string]*serviceLife)
	for digest, entry := range r.entries {
		if entry.current != nil && entry.current.client != nil {
			running[digest] = entry.current
		}
	}
	r.work.Add(1)
	r.mu.Unlock()
	answer := make(chan error, 1)
	go func() {
		defer r.work.Done()
		answer <- r.releaseConversation(conversation, running)
	}()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case err := <-answer:
		return err
	}
}

// StopAll stops every service and waits until each has ended, as connection loss does.
func (h *ServiceHandle) StopAll(ctx context.Context) error {
	r := h.registry
	r.mu.Lock()
	lives := make([]*serviceLife, 0)
	for _, entry := range r.entries {
		if entry.current != nil {
			lives = append(lives, entry.current)
			entry.current = nil
		}
	}
	r.mu.Unlock()
	for _, life := range lives {
		life.cancel()
	}
	for _, life := range lives {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-life.done:
		}
	}
	return nil
}

// ServiceLease claims one digest's service until explicitly released.
type ServiceLease struct{ release func() }

// Release ends the claim exactly once; repeated calls do nothing.
func (l *ServiceLease) Release() { l.release() }

// Invoking is a registered invocation whose release cancels its artifact waits.
type Invoking struct {
	registry *ServiceRegistry
	id, pkg  string
	resolver ArtifactResolver
	ctx      context.Context
	cancel   context.CancelFunc
	once     sync.Once
}

// Release unregisters the invocation exactly once and cancels its artifact waits.
func (i *Invoking) Release() {
	i.once.Do(func() {
		i.cancel()
		r := i.registry
		r.mu.Lock()
		defer r.mu.Unlock()
		if r.invocations[i.id] == i {
			delete(r.invocations, i.id)
		}
	})
}

// Resident is a running service a caller invokes. It does not replace a service lease.
type Resident struct{ life *serviceLife }

// Client returns the service's shared client. The registry owns and closes it.
func (r *Resident) Client() *cmdsdk.Client { return r.life.client }

// Failure resolves a transport failure to the service's exit and stderr tail when
// the connection was lost, waiting for its process to be reaped.
func (r *Resident) Failure(ctx context.Context, err error) error {
	if !connectionLost(err) {
		select {
		case <-r.life.done:
		case <-r.life.connectionEnded:
		default:
			return err
		}
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-r.life.done:
		return r.life.err
	}
}

type releaseResult struct {
	digest string
	life   *serviceLife
	err    error
}

// leaseLocked and considerLocked run under the registry mutex; they never wait on IO.
func (r *ServiceRegistry) leaseLocked(digest string) *ServiceLease {
	entry := r.entries[digest]
	if entry == nil {
		entry = &serviceEntry{}
		r.entries[digest] = entry
	}
	entry.leases++
	entry.changes++
	return &ServiceLease{release: sync.OnceFunc(func() {
		r.mu.Lock()
		var report func()
		defer func() {
			r.mu.Unlock()
			if report != nil {
				report()
			}
		}()
		current := r.entries[digest]
		if current == nil {
			return
		}
		current.leases--
		current.changes++
		report = r.considerLocked(digest, current)
	})}
}

func (r *ServiceRegistry) considerLocked(digest string, entry *serviceEntry) (report func()) {
	if r.closed {
		return
	}
	if entry.leases > 0 {
		r.decisions.add(digest, Leased)
		return
	}
	life := entry.current
	if life == nil {
		delete(r.entries, digest)
		return
	}
	if life.client == nil {
		delete(r.entries, digest)
		r.decisions.add(digest, Stops)
		return func() {
			slog.Info("service " + life.service + " is no longer needed and its start stops")
			life.cancel()
		}
	}
	if life.checking {
		return
	}
	life.checking = true
	changes := entry.changes
	r.decisions.add(digest, Asks)
	r.work.Add(1)
	go r.checkConversations(digest, entry, life, changes)
	return nil
}

// releaseConversation retires failed services before reconsidering residency for every entry.
func (r *ServiceRegistry) releaseConversation(conversation string, running map[string]*serviceLife) error {
	results := make(chan releaseResult, len(running))
	var work sync.WaitGroup
	for digest, life := range running {
		work.Add(1)
		go func() {
			defer work.Done()
			err := releaseConversation(r.stop, life.client, conversation)
			results <- releaseResult{digest, life, err}
		}()
	}
	work.Wait()
	close(results)
	var failures []error
	var retiring []*serviceLife
	r.mu.Lock()
	for result := range results {
		if result.err == nil {
			continue
		}
		failures = append(failures, result.err)
		entry := r.entries[result.digest]
		if entry != nil && entry.current == result.life {
			entry.current = nil
			retiring = append(retiring, result.life)
		}
	}
	r.mu.Unlock()
	for _, life := range retiring {
		life.cancel()
		<-life.done
	}
	var reports []func()
	r.mu.Lock()
	for digest, entry := range r.entries {
		entry.changes++
		if report := r.considerLocked(digest, entry); report != nil {
			reports = append(reports, report)
		}
	}
	r.mu.Unlock()
	for _, report := range reports {
		report()
	}
	return releaseFailures(failures)
}

// checkConversations asks outside the mutex and applies the answer only to the unchanged service lifetime.
func (r *ServiceRegistry) checkConversations(digest string, entry *serviceEntry, life *serviceLife, changes uint64) {
	defer r.work.Done()
	holds, err := serviceStatus(life.stop, life.client)
	r.mu.Lock()
	var report func()
	defer func() {
		r.mu.Unlock()
		if report != nil {
			report()
		}
	}()
	current := r.entries[digest]
	if r.closed || current != entry || current.current != life {
		return
	}
	life.checking = false
	switch {
	case entry.leases > 0:
		r.decisions.add(digest, Leased)
	case changes != entry.changes:
		report = r.considerLocked(digest, entry)
	case err != nil:
		report = func() {
			slog.Warn(
				"service " + life.service + " did not say which conversations it holds (" + err.Error() + "); it stays",
			)
		}
		r.decisions.add(digest, Unanswered)
	case holds:
		r.decisions.add(digest, HoldsConversations)
	default:
		report = func() {
			slog.Info("service " + life.service + " holds no lease or conversation and stops")
			life.cancel()
		}
		delete(r.entries, digest)
		r.decisions.add(digest, Stops)
	}
}
