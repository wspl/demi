package backend

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

const (
	// catalogFreshFor is how long a record stays fresh.
	catalogFreshFor = 15 * time.Minute
	// catalogRetryAfter is how long a failed refresh holds off the next
	// automatic one.
	catalogRetryAfter = time.Minute
	// catalogRefreshLimit is the longest a refresh may take.
	catalogRefreshLimit = 10 * time.Second
)

var (
	errCatalogCacheClosed      = errors.New("The model catalog cache is closed")
	errCatalogRefreshCancelled = errors.New("The catalog refresh was cancelled")
	errCatalogTimedOut         = errors.New("Model catalog request timed out")
)

// CatalogFetch reads a catalog's source; a refresh calls it when it starts,
// with a context that ends when the refresh is cancelled or runs out of
// time.
type CatalogFetch func(ctx context.Context) (core.ProviderModelList, error)

// ModelCatalogCache is the model catalog cache (models.md § Catalog cache):
// one validated record per entry, held in memory and in the control store's
// model_catalogs table, fresh for 15 minutes, refreshed once at a time by a
// goroutine of the cache's own that a reader who stops waiting does not
// cancel. A failed refresh keeps the last good record and holds off
// automatic refreshes for a minute.
type ModelCatalogCache struct {
	control *storage.Control
	clock   core.Clock
	// closing ends at Close, when stop is called; every entry's context is
	// its child.
	closing context.Context
	stop    context.CancelFunc
	// refreshes are the running refreshes; Close waits for them.
	refreshes *TaskGroup
	// mu guards entries: readers on any goroutine look entries up and never
	// wait under it.
	mu      sync.Mutex
	entries map[webapi.ProviderID]*catalogEntry
}

// catalogEntry is the cache of one entry under one key.
type catalogEntry struct {
	key string
	// ctx ends when the entry is invalidated, its key changes or the cache
	// closes; a refresh whose ctx ended writes nothing back.
	ctx    context.Context
	cancel context.CancelFunc
	// mu guards the fields below; it is never held across a wait.
	mu sync.Mutex
	// loaded says record was read from storage; a nil record then means
	// storage holds none under this entry's key.
	loaded  bool
	record  *storage.CatalogRecord
	failure *catalogFailure
	refresh *catalogRefresh
}

// catalogFailure is the last refresh's failure.
type catalogFailure struct {
	err error
	// retryAt is when automatic refreshes may start again.
	retryAt time.Time
}

// catalogRefresh is one running refresh, which its readers share: record
// and err are set before done closes.
type catalogRefresh struct {
	done   chan struct{}
	record *storage.CatalogRecord
	err    error
}

// NewModelCatalogCache keeps its records in control.
func NewModelCatalogCache(control *storage.Control, clock core.Clock) *ModelCatalogCache {
	closing, stop := context.WithCancel(context.Background())
	return &ModelCatalogCache{control: control, clock: clock, closing: closing, stop: stop, refreshes: newTaskGroup(closing), entries: map[webapi.ProviderID]*catalogEntry{}}
}

// Read is the catalog of entry id under key (models.md § Freshness and
// refresh): a fresh record at once; an expired one at once, marked stale,
// while one refresh starts or runs; without a record, or when force asks,
// what the refresh returns. fetch reads the source when a refresh starts. A
// failed refresh returns the kept record marked stale with the failure as a
// warning, or, without one, the failure. ctx bounds only this reader's
// wait.
func (c *ModelCatalogCache) Read(ctx context.Context, id webapi.ProviderID, key string, fetch CatalogFetch, force bool) (core.ProviderModelList, error) {
	entry, err := c.entry(id, key)
	if err != nil {
		return core.ProviderModelList{}, err
	}
	record, err := c.stored(ctx, id, entry)
	if err != nil {
		return core.ProviderModelList{}, err
	}
	entry.mu.Lock()
	// A refresh may have replaced the record while it loaded.
	if entry.record != nil {
		record = entry.record
	}
	if !force && entry.failure == nil && record != nil && c.fresh(record) {
		entry.mu.Unlock()
		return record.Catalog, nil
	}
	if failure := entry.failure; !force && failure != nil && time.Now().Before(failure.retryAt) {
		entry.mu.Unlock()
		if record == nil {
			return core.ProviderModelList{}, failure.err
		}
		return stale(record, failure.err), nil
	}
	refresh := entry.refresh
	if refresh == nil {
		refresh = c.start(id, entry, fetch)
		entry.refresh = refresh
	}
	if !force && record != nil {
		failure := entry.failure
		entry.mu.Unlock()
		if failure == nil {
			return stale(record, nil), nil
		}
		return stale(record, failure.err), nil
	}
	entry.mu.Unlock()
	select {
	case <-refresh.done:
	case <-ctx.Done():
		return core.ProviderModelList{}, ctx.Err()
	}
	if refresh.err == nil {
		return refresh.record.Catalog, nil
	}
	entry.mu.Lock()
	kept := entry.record
	entry.mu.Unlock()
	if kept == nil {
		return core.ProviderModelList{}, refresh.err
	}
	return stale(kept, refresh.err), nil
}

// Invalidate drops the entry's record from memory and storage, and cancels
// its refresh, which writes nothing back.
func (c *ModelCatalogCache) Invalidate(ctx context.Context, id webapi.ProviderID) error {
	c.mu.Lock()
	entry := c.entries[id]
	delete(c.entries, id)
	c.mu.Unlock()
	if entry != nil {
		entry.cancel()
		entry.mu.Lock()
		refresh := entry.refresh
		entry.mu.Unlock()
		// Its outcome belongs to the readers it cancels; waiting only
		// orders its last write before the deletion below.
		if refresh != nil {
			select {
			case <-refresh.done:
			case <-ctx.Done():
				return ctx.Err()
			}
		}
	}
	return c.control.DeleteCatalogRecord(ctx, id)
}

// Close cancels the running refreshes and waits for them.
func (c *ModelCatalogCache) Close() {
	c.stop()
	c.refreshes.close()
	c.refreshes.wait()
}

// entry is the entry id under key; a changed key starts the entry afresh
// and cancels the refresh of the old one.
func (c *ModelCatalogCache) entry(id webapi.ProviderID, key string) (*catalogEntry, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closing.Err() != nil {
		return nil, errCatalogCacheClosed
	}
	previous := c.entries[id]
	if previous != nil && previous.key == key {
		return previous, nil
	}
	if previous != nil {
		previous.cancel()
	}
	ctx, cancel := context.WithCancel(c.closing)
	entry := &catalogEntry{key: key, ctx: ctx, cancel: cancel}
	c.entries[id] = entry
	return entry, nil
}

// stored is the record storage holds under the entry's key, read once. A
// stored record that fails validation is the reader's failure, never
// repaired or dropped.
func (c *ModelCatalogCache) stored(ctx context.Context, id webapi.ProviderID, entry *catalogEntry) (*storage.CatalogRecord, error) {
	entry.mu.Lock()
	loaded, record := entry.loaded, entry.record
	entry.mu.Unlock()
	if loaded {
		return record, nil
	}
	stored, err := c.control.CatalogRecord(ctx, id)
	if err != nil {
		return nil, err
	}
	if stored != nil && stored.Key != entry.key {
		stored = nil
	}
	entry.mu.Lock()
	defer entry.mu.Unlock()
	// A concurrent read or a refresh may have loaded it meanwhile.
	if !entry.loaded {
		entry.loaded = true
		entry.record = stored
	}
	return entry.record, nil
}

// start starts a refresh of the entry as a goroutine of the cache.
func (c *ModelCatalogCache) start(id webapi.ProviderID, entry *catalogEntry, fetch CatalogFetch) *catalogRefresh {
	refresh := &catalogRefresh{done: make(chan struct{})}
	started := c.refreshes.Go(func(context.Context) { c.run(id, entry, fetch, refresh) })
	if !started {
		refresh.err = errCatalogCacheClosed
		close(refresh.done)
	}
	return refresh
}

// run reads the entry's source once within the limit and, when it answers
// a usable catalog, stores the new record and holds it in memory. A
// cancelled refresh writes nothing.
func (c *ModelCatalogCache) run(id webapi.ProviderID, entry *catalogEntry, fetch CatalogFetch, refresh *catalogRefresh) {
	ctx, cancel := context.WithTimeoutCause(entry.ctx, catalogRefreshLimit, errCatalogTimedOut)
	catalog, err := fetch(ctx)
	timedOut := errors.Is(context.Cause(ctx), errCatalogTimedOut)
	cancel()
	switch {
	case entry.ctx.Err() != nil:
		err = errCatalogRefreshCancelled
	case err != nil && timedOut:
		err = errCatalogTimedOut
	case err == nil:
		catalog, err = usable(catalog)
	}
	var record *storage.CatalogRecord
	if err == nil {
		record = &storage.CatalogRecord{Key: entry.key, CheckedAt: c.clock.Now(), Catalog: catalog}
		if entry.ctx.Err() != nil {
			err = errCatalogRefreshCancelled
		} else if stored := c.control.PutCatalogRecord(context.WithoutCancel(entry.ctx), id, *record); stored != nil {
			err = fmt.Errorf("The catalog could not be stored: %w", stored)
		}
	}
	if err != nil {
		record = nil
	}
	entry.mu.Lock()
	entry.refresh = nil
	if entry.ctx.Err() == nil {
		if err == nil {
			entry.loaded = true
			entry.record = record
			entry.failure = nil
		} else {
			entry.failure = &catalogFailure{err: err, retryAt: time.Now().Add(catalogRetryAfter)}
		}
	}
	entry.mu.Unlock()
	refresh.record = record
	refresh.err = err
	close(refresh.done)
}

// fresh says whether record was checked less than 15 minutes ago.
func (c *ModelCatalogCache) fresh(record *storage.CatalogRecord) bool {
	return c.clock.Now().Time().Sub(record.CheckedAt.Time()) < catalogFreshFor
}

// stale is the kept record marked stale, with failure among its warnings
// when there is one.
func stale(record *storage.CatalogRecord, failure error) core.ProviderModelList {
	catalog := record.Catalog
	catalog.Stale = true
	catalog.Warnings = slices.Clone(catalog.Warnings)
	if failure != nil {
		catalog.Warnings = append(catalog.Warnings, failure.Error())
	}
	return catalog
}

// usable is the catalog a source answered, when it can be kept: valid, and
// not a stale copy of the source's own, which counts as a failed refresh.
func usable(catalog core.ProviderModelList) (core.ProviderModelList, error) {
	if err := core.Validate(catalog); err != nil {
		return core.ProviderModelList{}, fmt.Errorf("The catalog cannot be read: %s", strings.TrimRightFunc(err.Error(), unicode.IsSpace))
	}
	if !catalog.Stale {
		return catalog, nil
	}
	if len(catalog.Warnings) == 0 {
		return core.ProviderModelList{}, errors.New("The provider answered a stale model catalog")
	}
	return core.ProviderModelList{}, errors.New(strings.Join(catalog.Warnings, "; "))
}
