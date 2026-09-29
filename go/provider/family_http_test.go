package provider_test

import (
	"context"
	"net/http"
	"net/url"
	"testing"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/anthropicapi"
	"github.com/wspl/demi/go/provider/google"
	"github.com/wspl/demi/go/provider/openaiapi"
	"github.com/wspl/demi/go/provider/providertest"
)

// Cost: loopback requests; the common lifecycle is checked at each API boundary.
func TestAPIKeyCatalogFailuresAndDroppedStreams(t *testing.T) {
	for _, family := range []string{"anthropic", "responses", "chat", "google"} {
		t.Run(family, func(t *testing.T) {
			for _, scenario := range []string{"refused", "broken", "dropped", "no-answer", "pre-cancelled", "cancelled"} {
				t.Run(scenario, func(t *testing.T) {
					frame := `{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"hello"}}`
					switch family {
					case "responses":
						frame = `{"type":"response.output_text.delta","delta":"hello"}`
					case "chat":
						frame = `{"choices":[{"delta":{"content":"hello"}}]}`
					case "google":
						frame = `{"candidates":[{"content":{"parts":[{"text":"hello"}]}}]}`
					}
					script := providertest.MockResponse{Chunks: []string{"data: " + frame + "\n\n"}, Ending: providertest.Open}
					switch scenario {
					case "refused":
						script = providertest.MockResponse{Status: 413, Headers: http.Header{"Retry-After": {"2"}}, Chunks: []string{"too large"}}
					case "broken":
						script.Ending = providertest.Broken
					}
					vendor := providertest.NewMockVendor(t, script)
					base, err := url.Parse(vendor.Server.URL)
					if err != nil {
						t.Fatal(err)
					}
					key, err := provider.NewSecret("key")
					if err != nil {
						t.Fatal(err)
					}
					var p provider.Provider
					switch family {
					case "anthropic":
						p, err = anthropicapi.New(anthropicapi.Config{APIKey: key, BaseURL: base}, providertest.FixedClock{})
					case "google":
						p, err = google.New(google.Config{APIKey: key, BaseURL: base}, providertest.FixedClock{})
					default:
						wire := core.WireAPIResponses
						if family == "chat" {
							wire = core.WireAPIChatCompletions
						}
						p, err = openaiapi.New(openaiapi.Config{APIKey: key, BaseURL: base, Wire: wire}, providertest.FixedClock{})
					}
					if err != nil {
						t.Fatal(err)
					}
					providertest.AssertBuiltInCatalog(t, p, vendor)
					runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: vendor.Server.Client()})
					if err != nil {
						t.Fatal(err)
					}
					if scenario == "no-answer" {
						vendor.Server.Close()
					}
					if scenario == "pre-cancelled" || scenario == "cancelled" {
						ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
						defer cancel()
						if scenario == "pre-cancelled" {
							cancel()
						}
						count := 0
						for event := range runtime.Run(ctx, providertest.InferenceRequest()) {
							if event != (provider.TextDelta{Text: "hello"}) {
								t.Fatalf("event after cancellation: %#v", event)
							}
							count++
							cancel()
						}
						if scenario == "pre-cancelled" {
							if count != 0 || len(vendor.Requests()) != 0 {
								t.Fatal("cancelled run sent a request")
							}
						} else {
							if count != 1 {
								t.Fatalf("events before cancellation: %d", count)
							}
							vendor.WaitDisconnect(t)
						}
						return
					}

					if scenario == "dropped" {
						ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
						defer cancel()
						count := 0
						for event := range runtime.Run(ctx, providertest.InferenceRequest()) {
							if event != (provider.TextDelta{Text: "hello"}) {
								t.Fatalf("first event %#v", event)
							}
							count++
							break
						}
						if count != 1 {
							t.Fatal("first event missing")
						}
						vendor.WaitDisconnect(t)
						return
					}
					events := providertest.Run(t, runtime, providertest.InferenceRequest())
					if len(events) == 0 {
						t.Fatal("missing failure")
					}
					failure, ok := events[len(events)-1].(provider.FailureEvent)
					if !ok {
						t.Fatalf("events %#v", events)
					}
					if scenario == "refused" {
						if failure.Failure.Code != provider.ContextLengthExceeded || *failure.Failure.RetryAfter != 2000 || provider.ReadHTTPFailureRecord(*failure.Failure.Diagnostics).Body != "too large" {
							t.Fatalf("refusal %#v", failure)
						}
					} else if failure.Failure.Code != provider.Overloaded {
						t.Fatalf("failure %#v", failure)
					}
					if scenario == "broken" && (len(events) != 2 || events[0] != (provider.TextDelta{Text: "hello"})) {
						t.Fatalf("broken stream %#v", events)
					}
				})
			}
		})
	}
}
