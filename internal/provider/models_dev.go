package provider

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"sync"
	"time"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

// ModelsDevURL is the published models.dev document address.
const ModelsDevURL = "https://models.dev/api.json"

// ModelsDevClient owns one conditionally revalidated document. Concurrent readers
// share a request; the last departing reader cancels and joins that request.
type ModelsDevClient struct {
	http   *http.Client
	url    string
	clock  core.Clock
	mu     sync.Mutex
	copy   *documentCopy
	flight *catalogFlight
}
type documentCopy struct {
	vendors     []ModelsDevVendor
	fetchedAt   core.Timestamp
	confirmedAt time.Time
	etag        string
	modified    string
}
type catalogFlight struct {
	done    chan struct{}
	cancel  context.CancelFunc
	readers int
	copy    *documentCopy
	err     error
}

// NewModelsDevClient returns a client with no cached document.
func NewModelsDevClient(client *http.Client, url string, clock core.Clock) *ModelsDevClient {
	return &ModelsDevClient{http: client, url: url, clock: clock}
}

// ModelsDevError means a request failed and no previous document was available.
type ModelsDevError struct {
	Message string
	Err     error
}

// Error returns the diagnostic for this failure.
func (e *ModelsDevError) Error() string { return e.Message }

// Unwrap returns the underlying cause.
func (e *ModelsDevError) Unwrap() error { return e.Err }

// ModelsDevSnapshot is one read's document and freshness metadata.
// Vendor values are returned as independent copies.
type ModelsDevSnapshot struct {
	vendors   []ModelsDevVendor
	FetchedAt core.Timestamp
	Stale     bool
	Warnings  []string
}

// Refreshed asks for the document again, conditionally when a copy exists.
func (c *ModelsDevClient) Refreshed(ctx context.Context) (ModelsDevSnapshot, error) {
	return c.read(ctx, false)
}

// Current reuses a copy downloaded or confirmed less than a day ago.
func (c *ModelsDevClient) Current(ctx context.Context) (ModelsDevSnapshot, error) {
	return c.read(ctx, true)
}

func (c *ModelsDevClient) read(ctx context.Context, reuse bool) (ModelsDevSnapshot, error) {
	if err := ctx.Err(); err != nil {
		return ModelsDevSnapshot{}, err
	}
	c.mu.Lock()
	if reuse && c.copy != nil && time.Since(c.copy.confirmedAt) < 24*time.Hour {
		result := c.copy.snapshot()
		c.mu.Unlock()
		return result, nil
	}
	flight := c.flight
	if flight == nil {
		workCtx, cancel := context.WithCancel(context.WithoutCancel(ctx))
		flight = &catalogFlight{done: make(chan struct{}), cancel: cancel}
		c.flight = flight
		previous := c.copy
		go func() {
			defer cancel()
			document, err := c.request(workCtx, previous)
			c.mu.Lock()
			flight.copy, flight.err = document, err
			if c.flight == flight {
				c.flight = nil
				if err == nil && workCtx.Err() == nil {
					c.copy = document
				}
			}
			close(flight.done)
			c.mu.Unlock()
		}()
	}
	flight.readers++
	c.mu.Unlock()
	return c.waitRead(ctx, flight)
}

func (c *ModelsDevClient) request(ctx context.Context, previous *documentCopy) (*documentCopy, error) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, c.url, nil)
	if err != nil {
		failure := RequestBuildFailure("models.dev", err)
		return nil, &ModelsDevError{Message: failure.Message, Err: err}
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
		failure := TransportFailure("models.dev", err)
		return nil, &ModelsDevError{Message: failure.Message, Err: err}
	}
	stop := closeOnCancel(ctx, response.Body)
	defer stop()
	if response.StatusCode == http.StatusNotModified && previous != nil {
		document := *previous
		document.confirmedAt = time.Now()
		return &document, nil
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, &ModelsDevError{
			Message: fmt.Sprintf("models.dev catalog request failed with HTTP %d", response.StatusCode),
		}
	}
	body, err := io.ReadAll(response.Body)
	if err != nil {
		failure := TransportFailure("models.dev", err)
		return nil, &ModelsDevError{Message: failure.Message, Err: err}
	}
	vendors, err := decodeModelsDocument(body)
	if err != nil {
		return nil, &ModelsDevError{Message: "models.dev catalog cannot be read at " + err.Error(), Err: err}
	}
	return &documentCopy{
		vendors:     vendors,
		fetchedAt:   c.clock.Now(),
		confirmedAt: time.Now(),
		etag:        response.Header.Get("ETag"),
		modified:    response.Header.Get("Last-Modified"),
	}, nil
}

func (c *documentCopy) snapshot() ModelsDevSnapshot {
	return ModelsDevSnapshot{vendors: c.vendors, FetchedAt: c.fetchedAt, Warnings: []string{}}
}

// ModelsDevVendor is one vendor's metadata and ordered model directory.
type ModelsDevVendor struct {
	ID     string  `json:"id"`
	Name   string  `json:"name"`
	NPM    *string `json:"npm"`
	API    *string `json:"api"`
	Doc    *string `json:"doc"`
	models []namedModel
	key    string
}
type namedModel struct {
	id    string
	model modelsDevModel
}
type modelsDevModel struct {
	Name             *string            `json:"name"`
	Description      *string            `json:"description"`
	Attachment       *bool              `json:"attachment"`
	Reasoning        *bool              `json:"reasoning"`
	ReasoningOptions *[]reasoningOption `json:"reasoning_options"`
	ToolCall         *bool              `json:"tool_call"`
	Limit            *modelLimit        `json:"limit"`
	Cost             *modelCost         `json:"cost"`
}
type reasoningOption struct {
	Kind   string     `json:"type"`
	Values *[]*string `json:"values"`
}
type modelLimit struct {
	Context *float64 `json:"context"`
	Output  *float64 `json:"output"`
}
type modelCost struct {
	Input      *float64 `json:"input"`
	Output     *float64 `json:"output"`
	CacheRead  *float64 `json:"cache_read"`
	CacheWrite *float64 `json:"cache_write"`
}

// vendorMembers preserves document order while reading each vendor object once.
func vendorMembers(data []byte, visit func(string, json.RawMessage) error) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	token, err := decoder.Token()
	if err != nil {
		return err
	}
	if token != json.Delim('{') {
		return errors.New("expected object")
	}
	for decoder.More() {
		start := decoder.InputOffset()
		token, err := decoder.Token()
		if err != nil {
			return err
		}
		_, ok := token.(string)
		if !ok {
			return errors.New("expected object key")
		}
		key := bytes.TrimLeft(data[start:decoder.InputOffset()], " \t\r\n,")
		name, err := contract.Decode[string](key)
		if err != nil {
			return err
		}
		var raw json.RawMessage
		if err := decoder.Decode(&raw); err != nil {
			return err
		}
		if err := visit(name, raw); err != nil {
			return err
		}
	}
	if _, err := decoder.Token(); err != nil {
		return err
	}
	if _, err := decoder.Token(); !errors.Is(err, io.EOF) {
		return errors.New("trailing JSON data")
	}
	return nil
}

func decodeModelsDocument(data []byte) ([]ModelsDevVendor, error) {
	vendors := make([]ModelsDevVendor, 0)
	err := vendorMembers(data, func(key string, raw json.RawMessage) error {
		wire, err := DecodeUntagged[struct {
			ID     string          `json:"id"`
			Name   string          `json:"name"`
			NPM    *string         `json:"npm"`
			API    *string         `json:"api"`
			Doc    *string         `json:"doc"`
			Models json.RawMessage `json:"models"`
		}](string(raw))
		if err != nil {
			return fmt.Errorf("%s: %w", key, err)
		}
		vendor := ModelsDevVendor{ID: wire.ID, Name: wire.Name, NPM: wire.NPM, API: wire.API, Doc: wire.Doc, key: key}
		vendor.models = make([]namedModel, 0)
		err = vendorMembers(wire.Models, func(id string, raw json.RawMessage) error {
			model, err := DecodeUntagged[modelsDevModel](string(raw))
			if err != nil {
				return fmt.Errorf("%s: %w", id, err)
			}
			for i := range vendor.models {
				if vendor.models[i].id == id {
					vendor.models[i].model = model
					return nil
				}
			}
			vendor.models = append(vendor.models, namedModel{id: id, model: model})
			return nil
		})
		if err != nil {
			return fmt.Errorf("%s.models: %w", key, err)
		}
		for i := range vendors {
			if vendors[i].key == key {
				vendors[i] = vendor
				return nil
			}
		}
		vendors = append(vendors, vendor)
		return nil
	})
	return vendors, err
}

// Vendors returns all vendors in document order. Each copy is independent.
func (s ModelsDevSnapshot) Vendors() []ModelsDevVendor {
	result := make([]ModelsDevVendor, len(s.vendors))
	for i, vendor := range s.vendors {
		result[i] = vendor.clone()
	}
	return result
}

// Vendor returns the vendor with the given document key.
func (s ModelsDevSnapshot) Vendor(id string) *ModelsDevVendor {
	for _, vendor := range s.vendors {
		if vendor.key == id {
			value := vendor.clone()
			return &value
		}
	}
	return nil
}

// clone isolates the public vendor metadata from cached state.
func (v ModelsDevVendor) clone() ModelsDevVendor {
	if v.NPM != nil {
		value := *v.NPM
		v.NPM = &value
	}
	if v.API != nil {
		value := *v.API
		v.API = &value
	}
	if v.Doc != nil {
		value := *v.Doc
		v.Doc = &value
	}
	return v
}

// VendorModels maps a vendor's entire directory onto a catalog with this read's metadata.
func (s ModelsDevSnapshot) VendorModels(id string) *core.ProviderModelList {
	vendor := s.Vendor(id)
	if vendor == nil {
		return nil
	}
	return &core.ProviderModelList{
		Models:          vendor.Models(),
		Warnings:        append([]string{}, s.Warnings...),
		SourceFetchedAt: s.FetchedAt,
		Stale:           s.Stale,
	}
}

// Models returns the vendor's models in document order with unknown facts kept absent.
func (v ModelsDevVendor) Models() []core.ProviderModel {
	result := make([]core.ProviderModel, 0, len(v.models))
	for _, named := range v.models {
		catalog := named.catalog()
		result = append(result, catalog)
	}
	return result
}

// modelTokens accepts positive integral token limits within uint32.
func modelTokens(value *float64) *uint32 {
	if value == nil || math.Trunc(*value) != *value || *value < 1 || *value > math.MaxUint32 {
		return nil
	}
	result := uint32(*value)
	return &result
}

func (c *ModelsDevClient) waitRead(ctx context.Context, flight *catalogFlight) (ModelsDevSnapshot, error) {
	select {
	case <-ctx.Done():
		c.mu.Lock()
		flight.readers--
		last := flight.readers == 0
		if last {
			flight.cancel()
			if c.flight == flight {
				c.flight = nil
			}
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
			return flight.copy.snapshot(), nil
		}
		if kept == nil {
			return ModelsDevSnapshot{}, flight.err
		}
		result := kept.snapshot()
		result.Stale = true
		result.Warnings = []string{"Using stale models.dev catalog: " + flight.err.Error()}
		return result, nil
	}
}

func (n namedModel) catalog() core.ProviderModel {
	// Return independently owned optional fields, without exposing the
	// cached model's pointers.
	model := n.model
	catalog := core.ProviderModel{ID: n.id, DisplayName: n.id, ServiceTiers: []core.ServiceTier{}}
	if model.Name != nil {
		catalog.DisplayName = *model.Name
	}
	if model.Description != nil {
		value := *model.Description
		catalog.Description = &value
	}
	if model.ToolCall != nil {
		value := *model.ToolCall
		catalog.SupportsTools = &value
	}
	if model.Attachment != nil {
		value := *model.Attachment
		catalog.SupportsAttachments = &value
	}
	if model.Reasoning != nil {
		value := *model.Reasoning
		catalog.SupportsReasoning = &value
	}
	if model.Limit != nil {
		catalog.ContextWindow = modelTokens(model.Limit.Context)
		catalog.OutputLimit = modelTokens(model.Limit.Output)
	}
	if model.Cost != nil {
		cost := core.ModelCost{}
		if model.Cost.Input != nil {
			value := *model.Cost.Input
			cost.Input = &value
		}
		if model.Cost.Output != nil {
			value := *model.Cost.Output
			cost.Output = &value
		}
		if model.Cost.CacheRead != nil {
			value := *model.Cost.CacheRead
			cost.CacheRead = &value
		}
		if model.Cost.CacheWrite != nil {
			value := *model.Cost.CacheWrite
			cost.CacheWrite = &value
		}
		catalog.Cost = &cost
	}
	catalog.SupportedThinkingEfforts = modelThinkingEfforts(model)
	return catalog
}

func modelThinkingEfforts(model modelsDevModel) *[]string {
	if model.ReasoningOptions != nil {
		for _, option := range *model.ReasoningOptions {
			if option.Kind != "effort" {
				continue
			}
			if option.Values != nil {
				efforts := make([]string, 0, len(*option.Values))
				for _, value := range *option.Values {
					if value != nil && *value != "" {
						efforts = append(efforts, *value)
					}
				}
				return &efforts
			}
			break
		}
	}
	return nil
}
