package google_test

import (
	"context"
	"encoding/json"
	"net/url"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/google"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"go.uber.org/goleak"
)

// These loopback scenarios require no external resources; the package budget is 10 seconds.
func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

// providerAt configures the Google entry against the scripted vendor.
func providerAt(t *testing.T, v *providertest.MockVendor, path string) *google.Provider {
	t.Helper()
	base, err := url.Parse(v.URL(path))
	if err != nil {
		t.Fatal(err)
	}
	key, err := provider.NewSecret("google-key")
	if err != nil {
		t.Fatal(err)
	}
	return google.New(google.Config{APIKey: key, BaseURL: base}, providertest.FixedClock("2026-01-01T00:00:00.000Z"))
}

// runtimeAt acquires a runtime and registers its release with the test.
func runtimeAt(t *testing.T, v *providertest.MockVendor, path string) provider.Runtime {
	t.Helper()
	r, err := providerAt(t, v, path).Runtime(provider.RuntimeEnv{HTTP: v.Client()})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := r.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return r
}

// bodyOf observes the request at the vendor boundary.
func bodyOf(t *testing.T, request provider.InferenceRequest) map[string]any {
	t.Helper()
	v := providertest.StartVendor(t)
	v.Respond(providertest.EventStream(""))
	events := providertest.Run(t.Context(), t, runtimeAt(t, v, "/v1beta"), request)
	if !reflect.DeepEqual([]provider.Event{&provider.Response{}}, events) {
		t.Fatalf("unexpected events: %#v", events)
	}
	return v.Requests()[0].JSON(t).(map[string]any)
}

// equalJSON compares a vendor payload with the expected wire value.
func equalJSON(t *testing.T, got any, expected string) {
	t.Helper()
	var want any
	if err := json.Unmarshal([]byte(expected), &want); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(want, got) {
		t.Fatalf("got %#v; want %#v", got, want)
	}
}

func TestBuiltinCatalog(t *testing.T) {
	v := providertest.StartVendor(t)
	p := providerAt(t, v, "/v1beta")
	providertest.AssertBuiltinCatalog(t.Context(), t, p, v)
	catalog, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if err := catalog.Validate(); err != nil {
		t.Fatal(err)
	}
	r := runtimeAt(t, v, "/v1beta")
	// The limits are the documented ones (docs/providers/models.md § Request limits).
	limits := r.RequestLimits(types.Model{})
	if *limits.BodyBytes != 20000000 || *limits.Images != 3600 {
		t.Fatalf("limits: %+v", limits)
	}
	fresh := r.Fresh()
	defer func() {
		if err := fresh.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	if !reflect.DeepEqual(limits, fresh.RequestLimits(types.Model{})) {
		t.Fatal("fresh runtime changed limits")
	}
}

func TestModelsEndpointAndKey(t *testing.T) {
	v := providertest.StartVendor(t)
	for _, base := range []string{"/v1beta", "/v1beta/"} {
		v.Respond(providertest.EventStream(""))
		request := providertest.InferenceRequest()
		request.ModelID = "gemini-3.6-flash"
		events := providertest.Run(t.Context(), t, runtimeAt(t, v, base), request)
		if !reflect.DeepEqual([]provider.Event{&provider.Response{}}, events) {
			t.Fatalf("unexpected events: %#v", events)
		}
	}
	for _, sent := range v.Requests() {
		if sent.URI != "/v1beta/models/gemini-3.6-flash:streamGenerateContent?alt=sse" || sent.Method != "POST" ||
			sent.Header("x-goog-api-key") != "google-key" ||
			sent.Header("accept") != "text/event-stream" ||
			sent.Header("content-type") != "application/json" {
			t.Fatalf("unexpected request: method=%s URI=%s", sent.Method, sent.URI)
		}
	}
}
