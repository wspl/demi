package providerhost_test

import (
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func TestVendorCatalogMapsOnlySupportedProtocolsAndDoesNotFallback(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		requests.Add(1)
		_, err := w.Write(
			[]byte(
				`{"anthropic":{"id":"anthropic","name":"Anthropic",` +
					`"npm":"@ai-sdk/anthropic","models":{}},"deepseek":{"id":"deepseek",` +
					`"name":"DeepSeek","npm":"@ai-sdk/openai-compatible","models":{}},` +
					`"openai":{"id":"openai","name":"OpenAI","npm":"@ai-sdk/openai",` +
					`"models":{}},"google":{"id":"google","name":"Google",` +
					`"npm":"@ai-sdk/google","models":{}},` +
					`"github-copilot":{"id":"github-copilot","name":"Copilot",` +
					`"npm":"@ai-sdk/openai","models":{}},"unsupported":{"id":"unsupported",` +
					`"name":"Unsupported","npm":"no-client","models":{}}}`,
			),
		)
		if err != nil {
			t.Error(err)
		}
	}))
	defer server.Close()
	vendors := providerhost.NewVendorCatalog(
		provider.NewModelsDevClient(server.Client(), server.URL, types.SystemClock{}),
	)
	list, err := vendors.Vendors(t.Context())
	if err != nil || len(list) != 4 {
		t.Fatal(list, err)
	}
	if list[0].ID != "anthropic" || list[1].ID != "deepseek" || list[2].ID != "google" || list[3].ID != "openai" {
		t.Fatal(list)
	}
	first, err := vendors.Vendor(t.Context(), "anthropic")
	if err != nil || first.BaseURL == nil || *first.BaseURL != "https://api.anthropic.com/v1" || first.WireAPI != nil {
		t.Fatal(first, err)
	}
	if requests.Load() != 1 {
		t.Fatal("vendor copy not reused")
	}
	deepseek := "deepseek"
	policy := vendors.Policy(&deepseek)
	if !policy.EffortAsBudget || !policy.PassBackReasoningContent {
		t.Fatal(policy)
	}
	anthropic := "anthropic"
	if vendors.Policy(&anthropic).EffortAsBudget || vendors.Policy(nil).EffortAsBudget {
		t.Fatal("first-party policy")
	}
	models, err := vendors.Models(t.Context(), "unsupported")
	if err != nil || len(models.Models) != 0 || models.SourceFetchedAt != types.UnixEpoch {
		t.Fatal(models, err)
	}
	if requests.Load() != 2 {
		t.Fatal("catalog did not refresh source")
	}
}
