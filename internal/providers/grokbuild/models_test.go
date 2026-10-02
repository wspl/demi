package grokbuild

import (
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestCatalogEnvelopeAndList(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt("/v1/models", answer(200, `{"object":"list","data":[{"id":"frontier","name":"Frontier","description":"frontier","context_window":500000,"supports_reasoning_effort":true,"reasoning_effort":"medium","reasoning_efforts":[{"id":"high","value":"high","default":true},{"id":"medium","default":false},{"value":"low"}]},{"model":"fast","context_window":200000},{"name":"no id"}]}`))
	v.RespondAt("/v1/models", answer(200, `[{"id":"grok-test"}]`))
	p, _ := fixture(t, v, nil)
	list, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, 2, len(list.Models))
	equal(t, "frontier", *list.DefaultModelID)
	equal(t, now, list.SourceFetchedAt)
	equal(t, false, list.Stale)
	frontier, fast := list.Models[0], list.Models[1]
	equal(t, "Frontier", frontier.DisplayName)
	equal(t, uint32(500000), *frontier.ContextWindow)
	equal(t, []string{"high", "medium", "low"}, *frontier.SupportedThinkingEfforts)
	equal(t, "high", *frontier.DefaultThinkingEffort)
	equal(t, true, *frontier.SupportsReasoning)
	equal(t, "fast", fast.DisplayName)
	if fast.SupportsReasoning != nil || fast.SupportedThinkingEfforts != nil {
		t.Fatal("invented reasoning capabilities")
	}
	for _, m := range list.Models {
		equal(t, true, *m.SupportsTools)
		equal(t, true, *m.SupportsAttachments)
	}
	equal(t, "Bearer session-token", v.Requests()[0].Header("authorization"))
	equal(t, "application/json", v.Requests()[0].Header("accept"))
	second, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, "grok-test", second.Models[0].ID)
	equal(t, 2, len(v.Requests()))
}
func TestCatalogFailures(t *testing.T) {
	v := providertest.StartVendor(t)
	p, _ := fixture(t, v, nil)
	for _, tc := range []struct {
		status  int
		body    string
		kind    provider.CatalogErrorKind
		message string
	}{{200, `{"data":[{"id":"frontier","context_window":"wide"}]}`, provider.CatalogInvalid, "context_window"}, {200, `{"data":[]}`, provider.CatalogInvalid, "Grok Build models answer lists no model"}, {503, "down", provider.CatalogUnavailable, "Grok Build models request failed with HTTP 503"}} {
		v.RespondAt("/v1/models", answer(tc.status, tc.body))
		_, err := p.ListModels(t.Context())
		var failure *provider.CatalogError
		if !errors.As(err, &failure) {
			t.Fatalf("unexpected error: %v", err)
		}
		equal(t, tc.kind, failure.Kind)
		if !strings.Contains(failure.Message, tc.message) {
			t.Fatal(failure.Message)
		}
	}
	staged := testProvider(v, provider.NewMemoryCredentialPool(), nil, v.Client())
	_, err := staged.ListModels(t.Context())
	equal(t, "No Grok account is signed in", err.Error())
}
