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
	return &ModelCatalogCache{control: control, clock: clock, entries: make(map[webapi.ProviderID]*catalogEntry), ctx: ctx, cancel: cancel}
}

// Read returns a fresh record, a stale record while refreshing, or waits when cold or forced.
func (c *ModelCatalogCache) Read(ctx context.Context, id webapi.ProviderID, key string, fetch CatalogFetch, force bool) (core.ProviderModelList, error) {
	c.mu.Lock()
	if c.ctx.Err() != nil {
		c.mu.Unlock()
		//nolint:staticcheck // Product error text is copied verbatim from Rust.
		return core.ProviderModelList{}, errors.New("The model catalog cache is closed")
	}
	e := c.entries[id]
	if e == nil || e.key != key {
		next := &catalogEntry{key: key}
		next.ctx, next.cancel = context.WithCancel(c.ctx)
		if e != nil {
			e.cancel()
			next.predecessor = e.refresh
			if next.predecessor == nil {
				next.predecessor = e.predecessor
			}
		}
		e = next
		c.entries[id] = e
	}
	loaded := e.loaded
	c.mu.Unlock()
	if !loaded {
		record, err := c.control.CatalogRecord(ctx, id)
		if err != nil {
			return core.ProviderModelList{}, err
		}
		if record != nil && record.Key != key {
			record = nil
		}
		c.mu.Lock()
		if !e.loaded {
			e.record = record
			e.loaded = true
		}
		c.mu.Unlock()
	}
	c.mu.Lock()
	if e.ctx.Err() != nil {
		c.mu.Unlock()
		//nolint:staticcheck // Product error text is copied verbatim from Rust.
		return core.ProviderModelList{}, errors.New("The catalog refresh was cancelled")
	}
	record := e.record
	if !force && e.failure == nil && catalogFresh(record, c.clock.Now()) {
		c.mu.Unlock()
		return catalogAnswer(record, false, nil)
	}
	if !force && e.failure != nil && time.Now().Before(e.retry) {
		failure := e.failure
		c.mu.Unlock()
		return catalogAnswer(record, true, failure)
	}
	r := e.refresh
	if r == nil {
		r = &catalogRefresh{done: make(chan struct{})}
		e.refresh = r
		c.workers.Go(func() { c.refresh(id, e, r, fetch) })
	}
	if !force && record != nil {
		failure := e.failure
		c.mu.Unlock()
		return catalogAnswer(record, true, failure)
	}
	c.mu.Unlock()
	select {
	case <-ctx.Done():
		return core.ProviderModelList{}, ctx.Err()
	case <-r.done:
	}
	if r.err == nil {
		return catalogAnswer(r.record, false, nil)
	}
	c.mu.Lock()
	record = e.record
	c.mu.Unlock()
	return catalogAnswer(record, true, r.err)
}

// Invalidate cancels the entry refresh and removes its cached record.
func (c *ModelCatalogCache) Invalidate(ctx context.Context, id webapi.ProviderID) error {
	c.mu.Lock()
	e := c.entries[id]
	delete(c.entries, id)
	var pending *catalogRefresh
	if e != nil {
		e.cancel()
		pending = e.refresh
		if pending == nil {
			pending = e.predecessor
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
func catalogAnswer(record *database.CatalogRecord, stale bool, failure error) (core.ProviderModelList, error) {
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
func (c *ModelCatalogCache) refresh(id webapi.ProviderID, e *catalogEntry, r *catalogRefresh, fetch CatalogFetch) {
	if e.predecessor != nil {
		<-e.predecessor.done
	}
	ctx, cancel := context.WithTimeout(e.ctx, 10*time.Second)
	defer cancel()
	var list core.ProviderModelList
	var err error
	if ctx.Err() != nil {
		err = ctx.Err()
	} else {
		list, err = fetch(ctx)
	}
	if e.ctx.Err() != nil {
		//nolint:staticcheck // Product error text is copied verbatim from Rust.
		err = errors.New("The catalog refresh was cancelled")
	} else if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		//nolint:staticcheck // Product error text is copied verbatim from Rust.
		err = errors.New("Model catalog request timed out")
	}
	if err == nil {
		if validation := list.Validate(); validation != nil {
			//nolint:staticcheck // Product error text is copied verbatim from Rust.
			err = fmt.Errorf("The catalog cannot be read: %w", validation)
		} else if list.Stale {
			message := strings.Join(list.Warnings, "; ")
			if message == "" {
				message = "The provider answered a stale model catalog"
			}
			err = errors.New(message)
		}
	}
	var record *database.CatalogRecord
	if err == nil {
		record = &database.CatalogRecord{Key: e.key, CheckedAt: c.clock.Now(), Catalog: list}
		// Freeze the source's mutable slices before publishing the cached record.
		record.Catalog, err = catalogAnswer(record, false, nil)
		if err == nil {
			err = c.control.PutCatalogRecord(e.ctx, id, *record)
			if err != nil {
				//nolint:staticcheck // Product error text is copied verbatim from Rust.
				err = fmt.Errorf("The catalog could not be stored: %w", err)
			}
		}
	}
	c.mu.Lock()
	e.refresh = nil
	if e.ctx.Err() == nil {
		if err == nil {
			e.record = record
			e.failure = nil
		} else {
			e.failure = err
			e.retry = time.Now().Add(time.Minute)
		}
	}
	r.record = record
	r.err = err
	close(r.done)
	c.mu.Unlock()
}
