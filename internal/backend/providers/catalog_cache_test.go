package providers

import (
	"context"
	"errors"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// cacheMemory replaces SQLite only for virtual-time scenarios. A separate test
// below verifies persistence and corrupt-record handling through the real store.
type cacheMemory struct {
	mu      sync.Mutex
	records map[webapi.ProviderID]database.CatalogRecord
	writes  int
}

func (s *cacheMemory) CatalogRecord(_ context.Context, id webapi.ProviderID) (*database.CatalogRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	record, ok := s.records[id]
	if !ok {
		return nil, nil
	}
	return &record, nil
}
func (s *cacheMemory) PutCatalogRecord(ctx context.Context, id webapi.ProviderID, r database.CatalogRecord) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return err
	}
	s.records[id] = r
	s.writes++
	return nil
}
func (s *cacheMemory) DeleteCatalogRecord(_ context.Context, id webapi.ProviderID) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.records, id)
	return nil
}
func memoryCache(t *testing.T, store *cacheMemory) *ModelCatalogCache {
	t.Helper()
	cache := NewModelCatalogCache(nil, core.SystemClock{})
	cache.control = store
	t.Cleanup(func() {
		if err := cache.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return cache
}
func catalog(name string) core.ProviderModelList {
	return core.ProviderModelList{Models: []core.ProviderModel{{ID: "model", DisplayName: name, ServiceTiers: []core.ServiceTier{}}}, Warnings: []string{}, SourceFetchedAt: core.UnixEpoch}
}
func answering(reads *atomic.Int32, list core.ProviderModelList, err error) CatalogFetch {
	return func(context.Context) (core.ProviderModelList, error) { reads.Add(1); return list, err }
}
func readCatalog(t *testing.T, c *ModelCatalogCache, key string, fetch CatalogFetch, force bool) core.ProviderModelList {
	t.Helper()
	list, err := c.Read(t.Context(), "provider", key, fetch, force)
	if err != nil {
		t.Fatal(err)
	}
	return list
}
func catalogName(list core.ProviderModelList) string { return list.Models[0].DisplayName }

func TestFreshCatalogServesMemoryAndStorageAndExpiredRefreshesOnce(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		store := &cacheMemory{records: make(map[webapi.ProviderID]database.CatalogRecord)}
		cache := memoryCache(t, store)
		var reads atomic.Int32
		var workers sync.WaitGroup
		for range 8 {
			workers.Go(func() {
				list := readCatalog(t, cache, "key", answering(&reads, catalog("First"), nil), false)
				if catalogName(list) != "First" || list.Stale {
					t.Error(list)
				}
			})
		}
		workers.Wait()
		readCatalog(t, cache, "key", answering(&reads, catalog("Other"), nil), false)
		if reads.Load() != 1 {
			t.Fatal("duplicate fetch", reads.Load())
		}
		if err := cache.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		cache = memoryCache(t, store)
		if got := readCatalog(t, cache, "key", answering(&reads, catalog("Other"), nil), false); catalogName(got) != "First" || reads.Load() != 1 {
			t.Fatal("restart refetched", got)
		}
		time.Sleep(15 * time.Minute)
		answer := make(chan struct{})
		started := make(chan struct{})
		fetch := func(ctx context.Context) (core.ProviderModelList, error) {
			reads.Add(1)
			close(started)
			select {
			case <-answer:
				return catalog("Updated"), nil
			case <-ctx.Done():
				return core.ProviderModelList{}, ctx.Err()
			}
		}
		for i := range 8 {
			var source CatalogFetch = fetch
			if i > 0 {
				source = answering(&reads, catalog("Never"), nil)
			}
			got := readCatalog(t, cache, "key", source, false)
			if !got.Stale || catalogName(got) != "First" {
				t.Fatal(got)
			}
		}
		<-started
		forced := make(chan core.ProviderModelList, 1)
		go func() { forced <- readCatalog(t, cache, "key", answering(&reads, catalog("Never"), nil), true) }()
		synctest.Wait()
		close(answer)
		got := <-forced
		if got.Stale || catalogName(got) != "Updated" || reads.Load() != 2 {
			t.Fatal(got, reads.Load())
		}
		record, err := store.CatalogRecord(t.Context(), "provider")
		if err != nil {
			t.Fatal(err)
		}
		if record.CheckedAt != cache.clock.Now() || catalogName(record.Catalog) != "Updated" {
			t.Fatal(record)
		}
	})
}

func TestFailedRefreshKeepsRecordAndHoldsOffUnlessForced(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		store := &cacheMemory{records: make(map[webapi.ProviderID]database.CatalogRecord)}
		cache := memoryCache(t, store)
		var reads, failures atomic.Int32
		readCatalog(t, cache, "key", answering(&reads, catalog("First"), nil), false)
		kept, _ := store.CatalogRecord(t.Context(), "provider")
		time.Sleep(15 * time.Minute)
		failed := answering(&failures, core.ProviderModelList{}, errors.New("offline"))
		got := readCatalog(t, cache, "key", failed, true)
		if !got.Stale || catalogName(got) != "First" || len(got.Warnings) != 1 || got.Warnings[0] != "offline" {
			t.Fatal(got)
		}
		stored, _ := store.CatalogRecord(t.Context(), "provider")
		if stored.CheckedAt != kept.CheckedAt {
			t.Fatal("failure replaced checked time")
		}
		readCatalog(t, cache, "key", failed, false)
		time.Sleep(59 * time.Second)
		readCatalog(t, cache, "key", failed, false)
		if failures.Load() != 1 {
			t.Fatal("retry during holdoff")
		}
		readCatalog(t, cache, "key", failed, true)
		if failures.Load() != 2 {
			t.Fatal("forced read held off")
		}
		got = readCatalog(t, cache, "key", answering(&reads, catalog("Recovered"), nil), true)
		if got.Stale || len(got.Warnings) != 0 || catalogName(got) != "Recovered" {
			t.Fatal(got)
		}
		readCatalog(t, cache, "key", failed, true)
		time.Sleep(time.Minute)
		readCatalog(t, cache, "key", failed, false)
		synctest.Wait()
		if failures.Load() != 4 {
			t.Fatal("automatic retry did not resume", failures.Load())
		}
	})
}

func TestChangedKeyAndInvalidationCancelRefreshWithoutWriting(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		store := &cacheMemory{records: make(map[webapi.ProviderID]database.CatalogRecord)}
		cache := memoryCache(t, store)
		var reads atomic.Int32
		started := make(chan struct{})
		stopped := make(chan struct{})
		result := make(chan error, 1)
		go func() {
			_, err := cache.Read(t.Context(), "provider", "old-account", func(ctx context.Context) (core.ProviderModelList, error) {
				reads.Add(1)
				close(started)
				<-ctx.Done()
				close(stopped)
				return catalog("Old"), nil
			}, false)
			result <- err
		}()
		<-started
		if err := cache.Invalidate(t.Context(), "provider"); err != nil {
			t.Fatal(err)
		}
		<-stopped
		if err := <-result; err == nil {
			t.Fatal("invalidated read succeeded")
		}
		if store.writes != 0 {
			t.Fatal("canceled refresh wrote")
		}
		got := readCatalog(t, cache, "new-account", answering(&reads, catalog("New"), nil), false)
		if catalogName(got) != "New" {
			t.Fatal(got)
		}
		got = readCatalog(t, cache, "changed-config", answering(&reads, catalog("Changed"), nil), false)
		if catalogName(got) != "Changed" || reads.Load() != 3 {
			t.Fatal(got, reads.Load())
		}
		stored, _ := store.CatalogRecord(t.Context(), "provider")
		if stored.Key != "changed-config" {
			t.Fatal(stored)
		}
		// A changed key also cancels an in-flight generation, not only a cached one.
		started = make(chan struct{})
		stopped = make(chan struct{})
		go func() {
			_, err := cache.Read(t.Context(), "provider", "pending", func(ctx context.Context) (core.ProviderModelList, error) {
				close(started)
				<-ctx.Done()
				close(stopped)
				return catalog("Late"), nil
			}, true)
			result <- err
		}()
		<-started
		got = readCatalog(t, cache, "replacement", answering(&reads, catalog("Replacement"), nil), false)
		<-stopped
		if err := <-result; err == nil {
			t.Fatal("changed generation succeeded")
		}
		if catalogName(got) != "Replacement" {
			t.Fatal(got)
		}
		if err := cache.Invalidate(t.Context(), "provider"); err != nil {
			t.Fatal(err)
		}
		stored, _ = store.CatalogRecord(t.Context(), "provider")
		if stored != nil {
			t.Fatal("invalidation kept record")
		}
	})
}

func TestColdFailuresInvalidAnswersTimeoutAndClose(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		store := &cacheMemory{records: make(map[webapi.ProviderID]database.CatalogRecord)}
		cache := memoryCache(t, store)
		var reads atomic.Int32
		invalid := catalog("Invalid")
		zero := uint32(0)
		invalid.Models[0].OutputLimit = &zero
		stale := catalog("Stale")
		stale.Stale = true
		stale.Warnings = []string{"Using stale models.dev catalog: offline"}
		cases := []struct {
			list core.ProviderModelList
			err  error
			want string
		}{{core.ProviderModelList{}, errors.New("offline"), "offline"}, {invalid, nil, "cannot be read"}, {stale, nil, stale.Warnings[0]}}
		for _, c := range cases {
			_, err := cache.Read(t.Context(), "provider", "key", answering(&reads, c.list, c.err), true)
			if err == nil || !strings.Contains(err.Error(), c.want) {
				t.Fatalf("%v, want %s", err, c.want)
			}
		}
		if store.writes != 0 {
			t.Fatal("unusable answer stored")
		}
		stopped := make(chan struct{})
		before := time.Now()
		_, err := cache.Read(t.Context(), "provider", "key", func(ctx context.Context) (core.ProviderModelList, error) {
			<-ctx.Done()
			close(stopped)
			return core.ProviderModelList{}, ctx.Err()
		}, true)
		<-stopped
		if err == nil || err.Error() != "Model catalog request timed out" || time.Since(before) != 10*time.Second {
			t.Fatal(err, time.Since(before))
		}
		started := make(chan struct{})
		result := make(chan error, 1)
		go func() {
			_, err := cache.Read(t.Context(), "provider", "key", func(ctx context.Context) (core.ProviderModelList, error) {
				close(started)
				<-ctx.Done()
				return core.ProviderModelList{}, ctx.Err()
			}, true)
			result <- err
		}()
		<-started
		if err := cache.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		if err := <-result; err == nil {
			t.Fatal("close left reader successful")
		}
		_, err = cache.Read(t.Context(), "provider", "key", answering(&reads, catalog("Late"), nil), false)
		if err == nil || err.Error() != "The model catalog cache is closed" {
			t.Fatal(err)
		}
	})
}

func TestCatalogRefreshOutlivesCanceledReader(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		store := &cacheMemory{records: make(map[webapi.ProviderID]database.CatalogRecord)}
		cache := memoryCache(t, store)
		ctx, cancel := context.WithCancel(t.Context())
		started := make(chan struct{})
		answer := make(chan struct{})
		result := make(chan error, 1)
		var reads atomic.Int32
		go func() {
			_, err := cache.Read(ctx, "provider", "key", func(ctx context.Context) (core.ProviderModelList, error) {
				reads.Add(1)
				close(started)
				select {
				case <-answer:
					return catalog("Finished"), nil
				case <-ctx.Done():
					return core.ProviderModelList{}, ctx.Err()
				}
			}, false)
			result <- err
		}()
		<-started
		cancel()
		if err := <-result; !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		// A second reader joins the existing owner task instead of starting another.
		joined := make(chan core.ProviderModelList, 1)
		go func() { joined <- readCatalog(t, cache, "key", answering(&reads, catalog("Wrong"), nil), false) }()
		synctest.Wait()
		close(answer)
		if got := <-joined; catalogName(got) != "Finished" || reads.Load() != 1 {
			t.Fatal(got, reads.Load())
		}
	})
}
