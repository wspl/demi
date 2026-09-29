package provider_test

import (
	"context"
	"errors"
	"net/http"
	"reflect"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/providertest"
)

const modelDocument = `{"key":{"id":"display-id","name":"Vendor","npm":"@ai-sdk/openai-compatible","api":"https://vendor.invalid","models":{"z":{"name":"Model Z","description":"Flagship","reasoning":true,"reasoning_options":[{"type":"budget"},{"type":"effort","values":[null,"low","","high"]}],"tool_call":true,"attachment":false,"limit":{"context":128000,"output":32000},"cost":{"input":0.3,"output":1.2,"cache_read":0.03}},"a":{"limit":{"context":128000.5,"output":0}}}},"second":{"id":"second","name":"Second","models":{}}}`

func TestModelsDevRevalidationCatalogAndStaleFallback(t *testing.T) {
	server := providertest.NewMockVendor(t,
		providertest.MockResponse{Status: 500},
		providertest.MockResponse{Status: 200, Chunks: []string{`{"vendor":{"id":"vendor","name":"Vendor","models":[]}}`}},
		providertest.MockResponse{Status: 200, Headers: http.Header{"Etag": {`"v1"`}, "Last-Modified": {"Mon, 07 Sep 2026 12:00:00 GMT"}}, Chunks: []string{modelDocument}},
		providertest.MockResponse{Status: 304}, providertest.MockResponse{Status: 304},
		providertest.MockResponse{Status: 503}, providertest.MockResponse{Status: 200, Chunks: []string{`[]`}})
	synctest.Test(t, func(t *testing.T) {
		defer server.Server.Client().CloseIdleConnections()
		client := provider.NewModelsDevClient(server.Server.Client(), server.Server.URL, providertest.FixedClock{})
		if _, err := client.Refreshed(t.Context()); err == nil || err.Error() != "models.dev catalog request failed with HTTP 500" {
			t.Fatal(err)
		}
		if _, err := client.Refreshed(t.Context()); err == nil || !strings.Contains(err.Error(), "vendor.models") {
			t.Fatal(err)
		}
		first, err := client.Refreshed(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		vendors := first.Vendors()
		if len(vendors) != 2 || vendors[0].ID != "display-id" || vendors[1].ID != "second" {
			t.Fatal(vendors)
		}
		list := first.VendorModels("key")
		if list == nil || len(list.Models) != 2 || first.VendorModels("missing") != nil {
			t.Fatal(list)
		}
		model := list.Models[0]
		if model.ID != "z" || model.DisplayName != "Model Z" || *model.ContextWindow != 128000 || *model.OutputLimit != 32000 || !*model.SupportsTools || *model.SupportsAttachments || !*model.SupportsReasoning || !reflect.DeepEqual(*model.SupportedThinkingEfforts, []string{"low", "high"}) || *model.Cost.Input != 0.3 {
			t.Fatalf("model: %+v", model)
		}
		bare := list.Models[1]
		if bare.ID != "a" || bare.DisplayName != "a" || bare.ContextWindow != nil || bare.OutputLimit != nil || bare.SupportsTools != nil || bare.Cost != nil {
			t.Fatal(bare)
		}
		if _, err := client.Current(t.Context()); err != nil {
			t.Fatal(err)
		}
		if len(server.Requests()) != 3 {
			t.Fatal("current refetched")
		}
		confirmed, err := client.Refreshed(t.Context())
		if err != nil || confirmed.FetchedAt != first.FetchedAt || confirmed.Stale {
			t.Fatal(confirmed, err)
		}
		request := server.Requests()[3]
		if request.Headers.Get("If-None-Match") != `"v1"` || request.Headers.Get("If-Modified-Since") != "Mon, 07 Sep 2026 12:00:00 GMT" || request.Headers.Get("Accept") != "application/json" {
			t.Fatal(request.Headers)
		}
		server.Server.Client().CloseIdleConnections()
		time.Sleep(24 * time.Hour) // Advance fake time to the catalog freshness boundary.
		if _, err := client.Current(t.Context()); err != nil {
			t.Fatal(err)
		}
		if len(server.Requests()) != 5 {
			t.Fatal("expired copy was reused")
		}
		stale, err := client.Refreshed(t.Context())
		if err != nil || !stale.Stale || stale.FetchedAt != first.FetchedAt || len(stale.Warnings) != 1 || stale.Warnings[0] != "Using stale models.dev catalog: models.dev catalog request failed with HTTP 503" {
			t.Fatal(stale, err)
		}
		if !stale.VendorModels("key").Stale {
			t.Fatal("catalog lost staleness")
		}
		malformed, err := client.Refreshed(t.Context())
		if err != nil || !malformed.Stale {
			t.Fatal(malformed, err)
		}
	})
}

func TestModelsDevConcurrentReadersAndCancellation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		release := make(chan struct{})
		vendor := providertest.NewPipeMockVendor(t, providertest.MockResponse{Chunks: []string{modelDocument}, Release: release}, providertest.MockResponse{Ending: providertest.Silent}, providertest.MockResponse{Chunks: []string{modelDocument}})
		client := provider.NewModelsDevClient(vendor.Server.Client(), vendor.Server.URL, providertest.FixedClock{})
		first, cancelFirst := context.WithCancel(t.Context())
		second, cancelSecond := context.WithCancel(t.Context())
		defer cancelFirst()
		defer cancelSecond()
		firstDone, secondDone := make(chan error, 1), make(chan error, 1)
		go func() { _, err := client.Current(first); firstDone <- err }()
		go func() { _, err := client.Current(second); secondDone <- err }()
		synctest.Wait()
		if len(vendor.Requests()) != 1 {
			t.Fatalf("concurrent reads sent %d requests", len(vendor.Requests()))
		}
		cancelFirst()
		if err := <-firstDone; !errors.Is(err, context.Canceled) {
			t.Fatalf("cancelled reader %v", err)
		}
		close(release)
		if err := <-secondDone; err != nil {
			t.Fatalf("remaining reader lost its fetch: %v", err)
		}
		abandoned, cancelAbandoned := context.WithCancel(t.Context())
		done := make(chan error, 1)
		go func() { _, err := client.Refreshed(abandoned); done <- err }()
		synctest.Wait()
		if len(vendor.Requests()) != 2 {
			t.Fatal("refresh did not start")
		}
		cancelAbandoned()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("last reader %v", err)
		}
		vendor.WaitDisconnect(t)
		if _, err := client.Refreshed(t.Context()); err != nil {
			t.Fatal(err)
		}
		if len(vendor.Requests()) != 3 {
			t.Fatal("cancelled flight was reused")
		}
	})
}
