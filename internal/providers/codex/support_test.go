package codex_test

import (
	"context"
	"encoding/json"
	"net/http"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/codex"
	"go.uber.org/goleak"
)

// These boundary scenarios use loopback or controlled IO; no vendor calls and no intentional wall-time waits.
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

const now core.Timestamp = "2026-09-18T14:00:00.000Z"

const (
	responses = "/backend-api/codex/responses"
	models    = "/backend-api/codex/models"
	account   = "cred-a"
)

func accessToken(t *testing.T, expiry int64) string {
	return providertest.JWT(
		t,
		map[string]any{"exp": expiry, "https://api.openai.com/auth": map[string]any{"chatgpt_account_id": "acct-1"}},
	)
}
func freshToken(t *testing.T) string { return accessToken(t, 1789740000+3600) }
func document(t *testing.T, access, refresh string, last core.Timestamp) string {
	t.Helper()
	id := providertest.JWT(
		t,
		map[string]any{
			"email":                       "dev@example.com",
			"https://api.openai.com/auth": map[string]any{"chatgpt_account_id": "acct-1"},
		},
	)
	b, err := provider.JSONBody(
		map[string]any{
			"accessToken":  access,
			"refreshToken": refresh,
			"idToken":      id,
			"accountId":    "acct-1",
			"lastRefresh":  last,
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func poolWith(t *testing.T, doc string) *provider.MemoryCredentialPool {
	t.Helper()
	pool := provider.NewMemoryCredentialPool()
	if err := pool.Write(
		t.Context(),
		provider.AccountMeta{ID: account, Label: "dev@example.com", UpdatedAt: now, Source: "test"},
		doc,
	); err != nil {
		t.Fatal(err)
	}
	if err := pool.SetActive(t.Context(), account); err != nil {
		t.Fatal(err)
	}
	return pool
}

func configured(
	t *testing.T,
	v *providertest.MockVendor,
	pool provider.CredentialPool,
	change func(*codex.Config),
) *codex.Provider {
	t.Helper()
	id := account
	config := codex.Config{Account: &id}
	config.BackendURL = v.URL("/backend-api")
	config.AuthURL = v.URL("")
	config.Transport = codex.SSE
	if change != nil {
		change(&config)
	}
	p, err := codex.New(config, pool, &provider.MemorySnapshots{}, v.Client(), providertest.FixedClock(now))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(v.Client().CloseIdleConnections)
	return p
}

func setup(t *testing.T) (*providertest.MockVendor, *provider.MemoryCredentialPool, *codex.Provider) {
	t.Helper()
	v := providertest.StartVendor(t)
	pool := poolWith(t, document(t, freshToken(t), "refresh-1", now))
	return v, pool, configured(t, v, pool, nil)
}

func run(
	ctx context.Context,
	t *testing.T,
	p *codex.Provider,
	client *http.Client,
	r provider.InferenceRequest,
) []provider.Event {
	t.Helper()
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: client})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := runtime.Close(ctx); err != nil {
			t.Error(err)
		}
	}()
	return providertest.Run(ctx, t, runtime, r)
}

func completed() providertest.MockResponse {
	return stream(`{"type":"response.completed",` +
		`"response":{"usage":{"input_tokens":1,"output_tokens":1}}}`)
}

func stream(frames ...string) providertest.MockResponse {
	text := ""
	for _, frame := range frames {
		text += "data: " + frame + "\n\n"
	}
	return providertest.EventStream(text)
}

func answer(status int, text string) providertest.MockResponse {
	return providertest.MockResponse{Status: status, Chunks: [][]byte{[]byte(text)}}
}

func equal(t *testing.T, got, want any) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v; want %#v", got, want)
	}
}

func failure(t *testing.T, events []provider.Event) provider.Failure {
	t.Helper()
	if len(events) != 1 {
		t.Fatalf("events: %#v", events)
	}
	e, ok := events[0].(*provider.Error)
	if !ok {
		t.Fatalf("event: %#v", events[0])
	}
	return e.Failure
}

func jsonObject(t *testing.T, b []byte) map[string]any {
	t.Helper()
	var result map[string]any
	if err := json.Unmarshal(b, &result); err != nil {
		t.Fatal(err)
	}
	return result
}
