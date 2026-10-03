package provider_test

import (
	"context"
	"errors"
	"io"
	"net/http"
	"os"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func modelDocument(t *testing.T) string {
	t.Helper()
	data, err := os.ReadFile("testdata/models_dev.json")
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

type roundTripper func(*http.Request) (*http.Response, error)

func (f roundTripper) RoundTrip(request *http.Request) (*http.Response, error) { return f(request) }
func TestModelsRevalidationAndSharedRequest(t *testing.T) {
	document := modelDocument(t)
	synctest.Test(t, func(t *testing.T) {
		entered := make(chan struct{})
		release := make(chan struct{})
		var mu sync.Mutex
		var requests []*http.Request
		transport := roundTripper(func(request *http.Request) (*http.Response, error) {
			mu.Lock()
			requests = append(requests, request)
			number := len(requests)
			mu.Unlock()
			if number == 1 {
				close(entered)
				select {
				case <-release:
				case <-request.Context().Done():
					return nil, request.Context().Err()
				}
				return &http.Response{
					StatusCode: 200,
					Header: http.Header{
						"Etag":          []string{`"v1"`},
						"Last-Modified": []string{"Mon, 07 Sep 2026 12:00:00 GMT"},
					},
					Body: io.NopCloser(strings.NewReader(document)),
				}, nil
			}
			return &http.Response{
				StatusCode: 304,
				Header:     http.Header{},
				Body:       io.NopCloser(strings.NewReader("")),
			}, nil
		})
		client := provider.NewModelsDevClient(
			&http.Client{Transport: transport},
			"https://fixture.invalid/api.json",
			providertest.FixedClock(now),
		)
		result := make(chan provider.ModelsDevSnapshot, 3)
		var workers sync.WaitGroup
		for i := range 3 {
			workers.Go(func() {
				var snapshot provider.ModelsDevSnapshot
				var err error
				if i == 2 {
					snapshot, err = client.Current(t.Context())
				} else {
					snapshot, err = client.Refreshed(t.Context())
				}
				if err != nil {
					t.Error(err)
				}
				result <- snapshot
			})
		}
		<-entered
		synctest.Wait()
		close(release)
		workers.Wait()
		first := <-result
		for range 2 {
			other := <-result
			requireEqual(t, other.FetchedAt, first.FetchedAt)
			requireEqual(t, other.Stale, false)
		}
		requireEqual(t, first.Stale, false)
		requireEqual(t, first.FetchedAt, now)
		requireEqual(t, first.Warnings, []string{})
		requireEqual(t, len(requests), 1)
		requireEqual(t, requests[0].Header.Get("Accept"), "application/json")
		if _, err := client.Current(t.Context()); err != nil {
			t.Fatal(err)
		}
		requireEqual(t, len(requests), 1)
		confirmed, err := client.Refreshed(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		requireEqual(t, len(requests), 2)
		requireEqual(t, requests[1].Header.Get("If-None-Match"), `"v1"`)
		requireEqual(t, requests[1].Header.Get("If-Modified-Since"), "Mon, 07 Sep 2026 12:00:00 GMT")
		requireEqual(t, confirmed.FetchedAt, first.FetchedAt)
		requireEqual(t, confirmed.Stale, false)
		list, _ := confirmed.VendorModels("deepseek")
		requireEqual(t, len(list.Models), 2)
		time.Sleep(24 * time.Hour) // Synctest advances the catalog's freshness boundary, not wall time.
		if _, err := client.Current(t.Context()); err != nil {
			t.Fatal(err)
		}
		requireEqual(t, len(requests), 3)
	})
}

func TestModelsStaleFallbackAndInitialFailure(t *testing.T) {
	vendor := providertest.StartVendor(t)
	client := provider.NewModelsDevClient(vendor.Client(), vendor.URL("/api.json"), providertest.FixedClock(now))
	vendor.Respond(providertest.MockResponse{Status: 500})
	_, err := client.Refreshed(t.Context())
	if err == nil || err.Error() != "models.dev catalog request failed with HTTP 500" {
		t.Fatalf("%v", err)
	}
	vendor.Respond(
		providertest.MockResponse{
			Status: 200,
			Chunks: [][]byte{[]byte(`{"vendor":{"id":"vendor","name":"Vendor","models":[]}}`)},
		},
	)
	_, err = client.Refreshed(t.Context())
	if err == nil || !strings.Contains(err.Error(), "vendor.models") {
		t.Fatalf("%v", err)
	}
	vendor.Respond(providertest.MockResponse{Status: 200, Chunks: [][]byte{[]byte(modelDocument(t))}})
	good, err := client.Refreshed(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	vendor.Respond(providertest.MockResponse{Status: 503})
	stale, err := client.Refreshed(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, stale.FetchedAt, good.FetchedAt)
	requireEqual(t, stale.Stale, true)
	requireEqual(
		t,
		stale.Warnings,
		[]string{"Using stale models.dev catalog: models.dev catalog request failed with HTTP 503"},
	)
	list, _ := stale.VendorModels("deepseek")
	requireEqual(t, list.Stale, true)
	requireEqual(t, list.Warnings, stale.Warnings)
	vendor.Respond(providertest.MockResponse{Status: 200, Chunks: [][]byte{[]byte(`[]`)}})
	stale, err = client.Refreshed(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, stale.Stale, true)
}

func TestModelsCatalogMapping(t *testing.T) {
	vendor := providertest.StartVendor(t)
	vendor.Respond(providertest.MockResponse{Status: 200, Chunks: [][]byte{[]byte(modelDocument(t))}})
	client := provider.NewModelsDevClient(vendor.Client(), vendor.URL("/api.json"), providertest.FixedClock(now))
	snapshot, err := client.Refreshed(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	vendors := snapshot.Vendors()
	requireEqual(t, len(vendors), 2)
	requireEqual(t, []string{vendors[0].ID, vendors[1].ID}, []string{"deepseek", "minimax"})
	requireEqual(t, []string{vendors[0].Name, vendors[1].Name}, []string{"DeepSeek", "MiniMax"})
	requireEqual(t, *vendors[0].NPM, "@ai-sdk/openai-compatible")
	if vendors[1].NPM != nil {
		t.Fatal("unknown npm became known")
	}
	listed, _ := snapshot.Vendor("deepseek")
	requireEqual(t, *listed.API, "https://api.deepseek.com")
	if _, ok := snapshot.VendorModels("amazon-bedrock"); ok {
		t.Fatal("unknown vendor became known")
	}
	list, _ := snapshot.VendorModels("deepseek")
	if list.DefaultModelID != nil || list.Stale {
		t.Fatalf("%+v", list)
	}
	requireEqual(t, list.SourceFetchedAt, snapshot.FetchedAt)
	description := "The flagship"
	contextWindow, outputLimit := uint32(128000), uint32(32000)
	yes, no := true, false
	efforts := []string{"low", "high"}
	input, output, cache := 0.3, 1.2, 0.03
	want := core.ProviderModel{
		ID:                       "deepseek-v4",
		DisplayName:              "DeepSeek V4",
		Description:              &description,
		ContextWindow:            &contextWindow,
		OutputLimit:              &outputLimit,
		SupportsTools:            &yes,
		SupportsAttachments:      &no,
		SupportsReasoning:        &yes,
		SupportedThinkingEfforts: &efforts,
		ServiceTiers:             []core.ServiceTier{},
		Cost:                     &core.ModelCost{Input: &input, Output: &output, CacheRead: &cache},
	}
	requireEqual(t, list.Models[0], want)
	requireEqual(
		t,
		list.Models[1],
		core.ProviderModel{
			ID:           "deepseek-v4-flash",
			DisplayName:  "deepseek-v4-flash",
			ServiceTiers: []core.ServiceTier{},
		},
	)
}

func TestModelsLastReaderCancelsAndJoins(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		started := make(chan struct{})
		ended := make(chan struct{})
		client := provider.NewModelsDevClient(
			&http.Client{Transport: roundTripper(func(request *http.Request) (*http.Response, error) {
				close(started)
				<-request.Context().Done()
				close(ended)
				return nil, request.Context().Err()
			})},
			"https://fixture.invalid/api.json",
			providertest.FixedClock(now),
		)
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		results := make(chan error, 1)
		go func() {
			_, err := client.Refreshed(ctx)
			results <- err
		}()
		<-started
		synctest.Wait()
		cancel()
		if err := <-results; !errors.Is(err, context.Canceled) {
			t.Fatalf("%v", err)
		}
		select {
		case <-ended:
		default:
			t.Fatal("request outlived last reader")
		}
	})
}

func TestModelsDuplicateKeysKeepFirstPositionAndLastValue(t *testing.T) {
	vendor := providertest.StartVendor(t)
	vendor.Respond(
		providertest.MockResponse{
			Status: 200,
			Chunks: [][]byte{
				[]byte(
					`{"first":{"id":"old","name":"Old","models":{}},` +
						`"second":{"id":"second","name":"Second","models":{}},` +
						`"first":{"id":"first","name":"First","models":{"a":{"name":"old"}` +
						`,"b":{},"a":{"name":"new"}}}}`,
				),
			},
		},
	)
	client := provider.NewModelsDevClient(vendor.Client(), vendor.URL("/api.json"), providertest.FixedClock(now))
	snapshot, err := client.Refreshed(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	vendors := snapshot.Vendors()
	requireEqual(t, len(vendors), 2)
	requireEqual(t, []string{vendors[0].ID, vendors[1].ID}, []string{"first", "second"})
	models := vendors[0].Models()
	requireEqual(t, len(models), 2)
	requireEqual(t, []string{models[0].ID, models[1].ID}, []string{"a", "b"})
	requireEqual(t, models[0].DisplayName, "new")
	models[0].DisplayName = "caller change"
	list, _ := snapshot.VendorModels("first")
	requireEqual(t, list.Models[0].DisplayName, "new")
}
