package grokbuild

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"go.uber.org/goleak"
)

// These boundary scenarios use only a loopback scripted vendor; no model calls.
// Ordinary cases cost milliseconds; login timers run in synctest time.
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

const now types.Timestamp = "2026-09-18T14:00:00.000Z"

const (
	chatPath = "/v1/chat/completions"
)

func fixture(
	t *testing.T,
	v *providertest.MockVendor,
	fields map[string]any,
) (*Provider, *provider.MemoryCredentialPool) {
	t.Helper()
	document := map[string]any{
		"accessToken":  "session-token",
		"refreshToken": "refresh-1",
		"expiresAt":    "2030-01-01T00:00:00.000Z",
		"issuer":       v.URL("/"),
		"clientId":     "client-1",
		"userId":       "user-1",
		"email":        "user@example.com",
	}
	for k, value := range fields {
		if value == nil {
			delete(document, k)
		} else {
			document[k] = value
		}
	}
	data, err := provider.JSONBody(document)
	if err != nil {
		t.Fatal(err)
	}
	pool := provider.NewMemoryCredentialPool()
	if err := pool.Write(
		t.Context(),
		provider.AccountMeta{ID: "cred-g", Label: "user@example.com", UpdatedAt: now},
		string(data),
	); err != nil {
		t.Fatal(err)
	}
	if err := pool.SetActive(t.Context(), "cred-g"); err != nil {
		t.Fatal(err)
	}
	account := "cred-g"
	return testProvider(v, pool, &account, v.Client()), pool
}

func testProvider(
	v *providertest.MockVendor,
	pool provider.CredentialPool,
	account *string,
	client *http.Client,
) *Provider {
	config := Config{Account: account}
	config.ProxyURL, _ = url.Parse(v.URL("/v1"))
	config.IssuerURL, _ = url.Parse(v.URL("/"))
	return New(config, pool, &provider.MemorySnapshots{}, client, providertest.FixedClock(now))
}

func answer(status int, text string) providertest.MockResponse {
	return providertest.MockResponse{
		Status:  status,
		Headers: http.Header{"Content-Type": {"application/json"}},
		Chunks:  [][]byte{[]byte(text)},
	}
}

func chat(payloads ...string) providertest.MockResponse {
	text := ""
	for _, p := range payloads {
		text += "data: " + p + "\n\n"
	}
	return providertest.EventStream(text + "data: [DONE]\n\n")
}

func run(t *testing.T, p *Provider, request provider.InferenceRequest) []provider.Event {
	t.Helper()
	r, err := p.Runtime(provider.RuntimeEnv{HTTP: p.http})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := r.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	return providertest.Run(t.Context(), t, r, request)
}

func stored(t *testing.T, pool *provider.MemoryCredentialPool) secret {
	t.Helper()
	entry, _, err := pool.Document("cred-g").Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	value, err := decodeSecret([]byte(entry.Text))
	if err != nil {
		t.Fatal(err)
	}
	return value
}

func equal(t *testing.T, want, got any) {
	t.Helper()
	if !reflect.DeepEqual(want, got) {
		t.Fatalf("want %#v; got %#v", want, got)
	}
}

func jsonValue(t *testing.T, text string) any {
	t.Helper()
	var value any
	if err := json.Unmarshal([]byte(text), &value); err != nil {
		t.Fatal(err)
	}
	return value
}

// loginTransport performs network IO outside the synctest bubble. Its channels
// are created outside too, so fake time cannot advance during vendor IO.
type loginTransport struct {
	requests  chan *http.Request
	responses chan loginResponse
}
type loginResponse struct {
	response *http.Response
	err      error
}

func (b *loginTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	b.requests <- r
	result := <-b.responses
	return result.response, result.err
}

func loginClient(t *testing.T, v *providertest.MockVendor) *http.Client {
	t.Helper()
	bridge := &loginTransport{requests: make(chan *http.Request), responses: make(chan loginResponse)}
	done := make(chan struct{})
	go func() {
		defer close(done)
		for request := range bridge.requests {
			response, err := v.Client().Do(request.WithContext(t.Context()))
			if err == nil {
				data, readErr := io.ReadAll(response.Body)
				_ = response.Body.Close() // The scripted response was fully consumed.
				if readErr != nil {
					err = readErr
				}
				response.Body = io.NopCloser(bytes.NewReader(data))
			}
			bridge.responses <- loginResponse{response, err}
		}
	}()
	t.Cleanup(func() {
		close(bridge.requests)
		<-done
	})
	return &http.Client{Transport: bridge}
}
