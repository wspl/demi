package provider

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"fmt"
	"io"
	"net/http"
	"slices"
	"sync"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
)

const ModelsDevDefaultURL = "https://models.dev/api.json"

type ModelsDevSnapshot struct {
	keys      []string
	vendors   []ModelsDevVendor
	FetchedAt core.Timestamp
	Stale     bool
	Warnings  []string
}

func (s ModelsDevSnapshot) Vendors() []ModelsDevVendor { return slices.Clone(s.vendors) }
func (s ModelsDevSnapshot) Vendor(id string) *ModelsDevVendor {
	for i, vendor := range s.vendors {
		if s.keys[i] == id {
			return &vendor
		}
	}
	return nil
}
func (s ModelsDevSnapshot) VendorModels(id string) *core.ProviderModelList {
	vendor := s.Vendor(id)
	if vendor == nil {
		return nil
	}
	return &core.ProviderModelList{Models: vendor.Models(), Warnings: slices.Clone(s.Warnings), SourceFetchedAt: s.FetchedAt, Stale: s.Stale}
}

type modelsDevCopy struct {
	snapshot       ModelsDevSnapshot
	confirmed      time.Time
	etag, modified string
}
type modelsDevFlight struct {
	done    chan struct{}
	cancel  context.CancelFunc
	readers int
	copy    *modelsDevCopy
	err     error
}

// ModelsDevClient shares one revalidated copy and one reference-counted fetch.
// Every fetch has waiting readers; cancellation of the last reader cancels and
// joins it. Successful copies are immutable after publication.
type ModelsDevClient struct {
	http   *http.Client
	url    string
	clock  core.Clock
	mu     sync.Mutex
	copy   *modelsDevCopy
	flight *modelsDevFlight
}

func NewModelsDevClient(client *http.Client, url string, clock core.Clock) *ModelsDevClient {
	return &ModelsDevClient{http: client, url: url, clock: clock}
}
func (c *ModelsDevClient) Current(ctx context.Context) (ModelsDevSnapshot, error) {
	return c.read(ctx, true)
}
func (c *ModelsDevClient) Refreshed(ctx context.Context) (ModelsDevSnapshot, error) {
	return c.read(ctx, false)
}
func (c *ModelsDevClient) read(ctx context.Context, reuse bool) (ModelsDevSnapshot, error) {
	if err := ctx.Err(); err != nil {
		return ModelsDevSnapshot{}, err
	}
	c.mu.Lock()
	if reuse && c.copy != nil && time.Since(c.copy.confirmed) < 24*time.Hour {
		snapshot := c.copy.snapshot
		c.mu.Unlock()
		return snapshot, nil
	}
	flight := c.flight
	if flight == nil {
		fetchCtx, cancel := context.WithCancel(context.WithoutCancel(ctx))
		flight = &modelsDevFlight{done: make(chan struct{}), cancel: cancel}
		c.flight = flight
		previous := c.copy
		go func() {
			result, err := c.request(fetchCtx, previous)
			c.mu.Lock()
			flight.copy = result
			flight.err = err
			if c.flight == flight {
				c.flight = nil
				if err == nil {
					c.copy = result
				}
			}
			close(flight.done)
			c.mu.Unlock()
			cancel()
		}()
	}
	flight.readers++
	c.mu.Unlock()
	select {
	case <-ctx.Done():
		c.mu.Lock()
		flight.readers--
		last := flight.readers == 0
		if last {
			if c.flight == flight {
				c.flight = nil
			}
			flight.cancel()
		}
		c.mu.Unlock()
		if last {
			<-flight.done
		}
		return ModelsDevSnapshot{}, ctx.Err()
	case <-flight.done:
		c.mu.Lock()
		flight.readers--
		kept := c.copy
		c.mu.Unlock()
		if flight.err == nil {
			return flight.copy.snapshot, nil
		}
		if kept == nil {
			return ModelsDevSnapshot{}, flight.err
		}
		snapshot := kept.snapshot
		snapshot.Stale = true
		snapshot.Warnings = []string{"Using stale models.dev catalog: " + flight.err.Error()}
		return snapshot, nil
	}
}
func (c *ModelsDevClient) request(ctx context.Context, previous *modelsDevCopy) (*modelsDevCopy, error) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, c.url, nil)
	if err != nil {
		return nil, fmt.Errorf("models.dev catalog request could not be built")
	}
	request.Header.Set("Accept", "application/json")
	if previous != nil {
		if previous.etag != "" {
			request.Header.Set("If-None-Match", previous.etag)
		}
		if previous.modified != "" {
			request.Header.Set("If-Modified-Since", previous.modified)
		}
	}
	response, err := c.http.Do(request)
	if err != nil {
		return nil, TransportFailure("models.dev", err)
	}
	defer response.Body.Close()
	if response.StatusCode == http.StatusNotModified && previous != nil {
		copy := *previous
		copy.confirmed = time.Now()
		return &copy, nil
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, fmt.Errorf("models.dev catalog request failed with HTTP %d", response.StatusCode)
	}
	body, err := io.ReadAll(response.Body)
	if err != nil {
		return nil, TransportFailure("models.dev", err)
	}
	vendors, keys, err := decodeModelsDev(body)
	if err != nil {
		return nil, fmt.Errorf("models.dev catalog cannot be read: %w", wire.Refusal(err))
	}
	return &modelsDevCopy{snapshot: ModelsDevSnapshot{vendors: vendors, keys: keys, FetchedAt: c.clock.Now(), Warnings: []string{}}, confirmed: time.Now(), etag: response.Header.Get("ETag"), modified: response.Header.Get("Last-Modified")}, nil
}

// decodeModelsDev keeps the document's vendor order while applying each vendor's schema.
func decodeModelsDev(body []byte) ([]ModelsDevVendor, []string, error) {
	dec := jsontext.NewDecoder(bytes.NewReader(body))
	if err := wire.BeginObject(dec); err != nil {
		return nil, nil, err
	}
	vendors := []ModelsDevVendor{}
	keys := []string{}
	for {
		name, more, err := wire.NextMember(dec)
		if err != nil {
			return nil, nil, err
		}
		if !more {
			break
		}
		var vendor ModelsDevVendor
		if err := vendor.UnmarshalJSONFrom(dec); err != nil {
			return nil, nil, wire.In(name, err)
		}
		// Lookups use the document key, as models.dev does, rather than the display id.
		vendors = append(vendors, vendor)
		keys = append(keys, name)
	}
	if _, err := dec.ReadToken(); err != io.EOF {
		if err != nil {
			return nil, nil, err
		}
		return nil, nil, fmt.Errorf("trailing JSON value")
	}
	return vendors, keys, nil
}
