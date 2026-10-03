package providers

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ModelCatalogCache holds validated catalogs in memory and in the control store.
// Close cancels and joins all refreshes, independently of individual readers.
type ModelCatalogCache struct {
	control catalogStore
	clock   core.Clock
	// mu protects entry state and worker admission, never fetches or storage IO.
	mu      sync.Mutex
	entries map[webapi.ProviderID]*catalogEntry
	ctx     context.Context
	cancel  context.CancelFunc
	workers sync.WaitGroup
}
type catalogStore interface {
	CatalogRecord(context.Context, webapi.ProviderID) (*database.CatalogRecord, error)
	PutCatalogRecord(context.Context, webapi.ProviderID, database.CatalogRecord) error
	DeleteCatalogRecord(context.Context, webapi.ProviderID) error
}
type catalogEntry struct {
	key     string
	ctx     context.Context
	cancel  context.CancelFunc
	loaded  bool
	record  *database.CatalogRecord
	failure error
	retry   time.Time
	refresh *catalogRefresh
	// predecessor orders a canceled generation's last write before this generation.
	predecessor *catalogRefresh
}
type catalogRefresh struct {
	done   chan struct{}
	record *database.CatalogRecord
	err    error
}

// NewModelCatalogCache creates a cache whose refreshes are owned until Close.
func NewModelCatalogCache(control *database.ControlService, clock core.Clock) *ModelCatalogCache {
	ctx, cancel := context.WithCancel(context.Background())
	return &ModelCatalogCache{
		control: control,
		clock:   clock,
		entries: make(map[webapi.ProviderID]*catalogEntry),
		ctx:     ctx,
		cancel:  cancel,
	}
}

// Read returns a fresh record, a stale record while refreshing, or waits when cold or forced.
func (c *ModelCatalogCache) Read(
	ctx context.Context,
	id webapi.ProviderID,
	key string,
	fetch CatalogFetch,
	force bool,
) (core.ProviderModelList, error) {
	c.mu.Lock()
	if c.ctx.Err() != nil {
		c.mu.Unlock()
		//nolint:staticcheck // Product error text, shown to the user as it is.
		return core.ProviderModelList{}, errors.New("The model catalog cache is closed")
	}
	entry := c.entryLocked(id, key)
	loaded := entry.loaded
	c.mu.Unlock()
	if !loaded {
		if err := c.loadEntry(ctx, id, key, entry); err != nil {
			return core.ProviderModelList{}, err
		}
	}
	c.mu.Lock()
	if entry.ctx.Err() != nil {
		c.mu.Unlock()
		//nolint:staticcheck // Product error text, shown to the user as it is.
		return core.ProviderModelList{}, errors.New("The catalog refresh was cancelled")
	}
	record := entry.record
	if !force && entry.failure == nil && catalogFresh(record, c.clock.Now()) {
		c.mu.Unlock()
		return catalogAnswer(record, false, nil)
	}
	if !force && entry.failure != nil && time.Now().Before(entry.retry) {
		failure := entry.failure
		c.mu.Unlock()
		return catalogAnswer(record, true, failure)
	}
	refresh := entry.refresh
	if refresh == nil {
		refresh = &catalogRefresh{done: make(chan struct{})}
		entry.refresh = refresh
		c.workers.Go(func() {
			c.refresh(id, entry, refresh, fetch)
		})
	}
	if !force && record != nil {
		failure := entry.failure
		c.mu.Unlock()
		return catalogAnswer(record, true, failure)
	}
	c.mu.Unlock()
	return c.waitCatalogRefresh(ctx, entry, refresh)
}

// Invalidate cancels the entry refresh and removes its cached record.
func (c *ModelCatalogCache) Invalidate(ctx context.Context, id webapi.ProviderID) error {
	c.mu.Lock()
	entry := c.entries[id]
	delete(c.entries, id)
	var pending *catalogRefresh
	if entry != nil {
		entry.cancel()
		pending = entry.refresh
		if pending == nil {
			pending = entry.predecessor
		}
	}
	c.mu.Unlock()
	// Invalidation is a commit: finish ordering the old write and row deletion.
	if pending != nil {
		<-pending.done
	}
	return c.control.DeleteCatalogRecord(context.WithoutCancel(ctx), id)
}

// Close cancels and joins all refreshes.
func (c *ModelCatalogCache) Close(_ context.Context) error {
	c.mu.Lock()
	c.cancel()
	c.mu.Unlock()
	c.workers.Wait()
	return nil
}

func catalogFresh(record *database.CatalogRecord, now core.Timestamp) bool {
	if record == nil {
		return false
	}
	checked, err := record.CheckedAt.Time()
	if err != nil {
		return false
	}
	instant, err := now.Time()
	return err == nil && instant.Sub(checked) < 15*time.Minute
}

// catalogAnswer gives the caller an independent catalog, retaining stale warnings.
func catalogAnswer(
	record *database.CatalogRecord,
	stale bool,
	failure error,
) (core.ProviderModelList, error) {
	if record == nil {
		return core.ProviderModelList{}, failure
	}
	data, err := contract.EncodeJSON(record.Catalog)
	if err != nil {
		return core.ProviderModelList{}, err
	}
	list, err := core.DecodeProviderModelList(data)
	if err != nil {
		return list, err
	}
	if stale {
		list.Stale = true
	}
	if failure != nil {
		list.Warnings = append(list.Warnings, failure.Error())
	}
	return list, nil
}

func (c *ModelCatalogCache) refresh(
	id webapi.ProviderID,
	entry *catalogEntry,
	refresh *catalogRefresh,
	fetch CatalogFetch,
) {
	if entry.predecessor != nil {
		<-entry.predecessor.done
	}
	ctx, cancel := context.WithTimeout(entry.ctx, 10*time.Second)
	defer cancel()
	var list core.ProviderModelList
	var err error
	if ctx.Err() != nil {
		err = ctx.Err()
	} else {
		list, err = fetch(ctx)
	}
	if entry.ctx.Err() != nil {
		//nolint:staticcheck // Product error text, shown to the user as it is.
		err = errors.New("The catalog refresh was cancelled")
	} else if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		//nolint:staticcheck // Product error text, shown to the user as it is.
		err = errors.New("Model catalog request timed out")
	}
	if err == nil {
		err = checkFetchedCatalog(list)
	}
	var record *database.CatalogRecord
	if err == nil {
		record = &database.CatalogRecord{Key: entry.key, CheckedAt: c.clock.Now(), Catalog: list}
		// Freeze the source's mutable slices before publishing the cached record.
		record.Catalog, err = catalogAnswer(record, false, nil)
		if err == nil {
			err = c.control.PutCatalogRecord(entry.ctx, id, *record)
			if err != nil {
				//nolint:staticcheck // Product error text, shown to the user as it is.
				err = fmt.Errorf("The catalog could not be stored: %w", err)
			}
		}
	}
	c.mu.Lock()
	entry.refresh = nil
	if entry.ctx.Err() == nil {
		if err == nil {
			entry.record = record
			entry.failure = nil
		} else {
			entry.failure = err
			entry.retry = time.Now().Add(time.Minute)
		}
	}
	refresh.record = record
	refresh.err = err
	close(refresh.done)
	c.mu.Unlock()
}

// entryLocked replaces a catalog generation while ordering its predecessor’s writes.
func (c *ModelCatalogCache) entryLocked(id webapi.ProviderID, key string) *catalogEntry {
	entry := c.entries[id]
	if entry == nil || entry.key != key {
		next := &catalogEntry{key: key}
		next.ctx, next.cancel = context.WithCancel(c.ctx)
		if entry != nil {
			entry.cancel()
			next.predecessor = entry.refresh
			if next.predecessor == nil {
				next.predecessor = entry.predecessor
			}
		}
		entry = next
		c.entries[id] = entry
	}
	return entry
}

// checkFetchedCatalog refuses invalid or stale provider catalogs.
func checkFetchedCatalog(list core.ProviderModelList) error {
	var err error

	if validation := list.Validate(); validation != nil {
		//nolint:staticcheck // Product error text, shown to the user as it is.
		err = fmt.Errorf("The catalog cannot be read: %w", validation)
	} else if list.Stale {
		message := strings.Join(list.Warnings, "; ")
		if message == "" {
			message = "The provider answered a stale model catalog"
		}
		err = errors.New(message)
	}
	return err
}

// loadEntry reads a catalog record before publishing it under the cache mutex.
func (c *ModelCatalogCache) loadEntry(
	ctx context.Context,
	id webapi.ProviderID,
	key string,
	entry *catalogEntry,
) error {
	record, err := c.control.CatalogRecord(ctx, id)
	if err != nil {
		return err
	}
	if record != nil && record.Key != key {
		record = nil
	}
	c.mu.Lock()
	if !entry.loaded {
		entry.record = record
		entry.loaded = true
	}
	c.mu.Unlock()
	return nil
}

// waitCatalogRefresh waits for a refresh and returns the current stale record if it fails.
func (c *ModelCatalogCache) waitCatalogRefresh(
	ctx context.Context,
	entry *catalogEntry,
	refresh *catalogRefresh,
) (core.ProviderModelList, error) {
	select {
	case <-ctx.Done():
		return core.ProviderModelList{}, ctx.Err()
	case <-refresh.done:
	}
	if refresh.err == nil {
		return catalogAnswer(refresh.record, false, nil)
	}
	c.mu.Lock()
	record := entry.record
	c.mu.Unlock()
	return catalogAnswer(record, true, refresh.err)
}
