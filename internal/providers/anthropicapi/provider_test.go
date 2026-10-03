package anthropicapi_test

import (
	"context"
	"encoding/json"
	"net/url"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/anthropicapi"
	"go.uber.org/goleak"
)

// Tests use only a loopback scripted vendor; each run costs one local request.
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

const now = "2026-09-18T14:00:00.000Z"

// providerAt builds an entry pointed at the scripted vendor.
func providerAt(t *testing.T, v *providertest.MockVendor, base string, policy provider.VendorPolicy) *anthropicapi.Provider {
	t.Helper()
	endpoint, err := url.Parse(v.URL(base))
	if err != nil {
		t.Fatal(err)
	}
	key, err := provider.NewSecret("sk-ant-test")
	if err != nil {
		t.Fatal(err)
	}
	return anthropicapi.New(anthropicapi.Config{APIKey: key, BaseURL: endpoint, Policy: policy}, providertest.FixedClock(now))
}

// testRuntime opens a Messages runtime owned by this test.
func testRuntime(t *testing.T, v *providertest.MockVendor, policy provider.VendorPolicy) provider.Runtime {
	t.Helper()
	r, err := providerAt(t, v, "/v1", policy).Runtime(provider.RuntimeEnv{HTTP: v.Client()})
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

// recorded frames vendor JSON payloads as SSE.
func recorded(payloads ...string) providertest.MockResponse {
	text := ""
	for _, payload := range payloads {
		text += "data: " + payload + "\n\n"
	}
	return providertest.EventStream(text)
}

// eventsOf runs one recorded vendor response.
func eventsOf(t *testing.T, response providertest.MockResponse) []provider.Event {
	t.Helper()
	v := providertest.StartVendor(t)
	v.Respond(response)
	return providertest.Run(t.Context(), t, testRuntime(t, v, provider.VendorPolicy{}), providertest.InferenceRequest())
}

// equalEvents compares observable run events.
func equalEvents(t *testing.T, got, want []provider.Event) {
	t.Helper()
	if !reflect.DeepEqual(want, got) {
		t.Fatalf("got %#v; want %#v", got, want)
	}
}

// sentBody runs a request and returns the JSON seen by the vendor.
func sentBody(t *testing.T, request provider.InferenceRequest, policy provider.VendorPolicy) map[string]any {
	t.Helper()
	v := providertest.StartVendor(t)
	v.Respond(recorded(`{"type":"message_stop"}`))
	events := providertest.Run(t.Context(), t, testRuntime(t, v, policy), request)
	equalEvents(t, events, []provider.Event{&provider.Response{}})
	return v.Requests()[0].JSON(t).(map[string]any)
}

// equalJSON compares the request's observable JSON without depending on key order.
func equalJSON(t *testing.T, got any, want string) {
	t.Helper()
	var expected any
	if err := json.Unmarshal([]byte(want), &expected); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(expected, got) {
		t.Fatalf("got %#v; want %#v", got, expected)
	}
}

func TestBuiltInDirectory(t *testing.T) {
	v := providertest.StartVendor(t)
	p := providerAt(t, v, "/v1", provider.VendorPolicy{})
	providertest.AssertBuiltinCatalog(t.Context(), t, p, v)
	list, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	ids := []string{}
	for _, m := range list.Models {
		ids = append(ids, m.ID)
		if *m.ContextWindow != 1000000 || !*m.SupportsTools || !*m.SupportsAttachments || *m.SupportsVideo || !*m.SupportsReasoning || *m.CanDisableThinking {
			t.Fatalf("incorrect model: %+v", m)
		}
	}
	if !reflect.DeepEqual([]string{"claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6", "claude-fable-5"}, ids) {
		t.Fatal(ids)
	}
	if p.Quota() != nil || p.Accounts() != nil {
		t.Fatal("API key has subscription state")
	}
	r := testRuntime(t, v, provider.VendorPolicy{})
	for _, window := range []uint32{200000, 1000000} {
		limits := r.RequestLimits(core.Model{ContextWindow: window})
		images := uint32(100)
		if window > 200000 {
			images = 600
		}
		if *limits.BodyBytes != 32000000 || *limits.Images != images {
			t.Fatal(limits)
		}
	}
	fresh := r.Fresh()
	if err := fresh.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
}
func TestStandardFailureReading(t *testing.T) {
	v := providertest.StartVendor(t)
	record := provider.NewHTTPFailureRecord(429, map[string][]string{"Retry-After": {"90"}}, "")
	raw, err := provider.JSONBody(record)
	if err != nil {
		t.Fatal(err)
	}
	upstream := string(raw)
	facts := providerAt(t, v, "/v1", provider.VendorPolicy{}).ReadFailure(&core.ProviderErrorDiagnostics{Source: "http", HTTPStatus: new(uint16(429)), Upstream: &upstream}, now)
	if facts.RetryAt == nil || *facts.RetryAt != "2026-09-18T14:01:30.000Z" {
		t.Fatal(facts)
	}
}
