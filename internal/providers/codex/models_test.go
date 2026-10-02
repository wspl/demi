package codex_test

import (
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

func catalogFixture(t *testing.T) string {
	t.Helper()
	b, err := os.ReadFile("testdata/models.json")
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}
func TestModelCatalog(t *testing.T) {
	v, _, p := setup(t)
	v.RespondAt(models, answer(200, catalogFixture(t)))
	list, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, len(list.Models), 3)
	equal(t, *list.DefaultModelID, "first")
	equal(t, list.SourceFetchedAt, now)
	equal(t, list.Stale, false)
	request := v.Requests()[0]
	equal(t, request.URI, models+"?client_version=0.153.4")
	equal(t, request.Header("Authorization"), "Bearer "+freshToken(t))
	equal(t, request.Header("Chatgpt-Account-Id"), "acct-1")
	equal(t, request.Header("Accept"), "application/json")
	gpt := list.Models[1]
	equal(t, gpt.ID, "gpt-5.5")
	equal(t, gpt.DisplayName, "GPT-5.5")
	equal(t, *gpt.ContextWindow, uint32(272000))
	equal(t, *gpt.SupportsTools, true)
	equal(t, *gpt.SupportsAttachments, true)
	equal(t, *gpt.SupportsReasoning, true)
	equal(t, *gpt.SupportedThinkingEfforts, []string{"low", "medium", "high", "xhigh", "ultra"})
	equal(t, *gpt.DefaultThinkingEffort, "medium")
	equal(t, *gpt.CanDisableThinking, false)
	description := "1.5x speed, increased usage"
	equal(t, gpt.ServiceTiers, []core.ServiceTier{{ID: "priority", Label: "Fast", Description: &description, Fast: true}})
	mini := list.Models[2]
	equal(t, mini.SupportsTools, (*bool)(nil))
	equal(t, *mini.SupportsAttachments, false)
	equal(t, *mini.SupportsReasoning, false)
	equal(t, *list.Models[0].SupportsTools, true)
	equal(t, *list.Models[0].DefaultServiceTierID, "priority")
}
func TestCatalogRefreshesOnce(t *testing.T) {
	v, _, p := setup(t)
	v.RespondAt(models, answer(401, "unauthorized"))
	v.RespondAt("/oauth/token", answer(200, `{"access_token":"new-access","refresh_token":"refresh-2"}`))
	v.RespondAt(models, answer(200, catalogFixture(t)))
	list, err := p.ListModels(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, len(list.Models), 3)
	equal(t, len(v.Requests()), 3)
	equal(t, v.Requests()[2].Header("Authorization"), "Bearer new-access")
}
func TestCatalogFailures(t *testing.T) {
	v, _, p := setup(t)
	fixture := catalogFixture(t)
	for _, test := range []struct {
		body   string
		status int
		kind   provider.CatalogErrorKind
		field  string
	}{
		{strings.Replace(fixture, `"effort":"low"`, `"effort":3`, 1), 200, provider.CatalogInvalid, "effort"},
		{strings.Replace(fixture, `"default_reasoning_level":"medium"`, `"default_reasoning_level":3`, 1), 200, provider.CatalogInvalid, "default_reasoning_level"},
		{"overloaded", 503, provider.CatalogUnavailable, "HTTP 503"},
	} {
		v.RespondAt(models, answer(test.status, test.body))
		_, err := p.ListModels(t.Context())
		var catalog *provider.CatalogError
		if !errors.As(err, &catalog) {
			t.Fatalf("error: %v", err)
		}
		equal(t, catalog.Kind, test.kind)
		if !strings.Contains(catalog.Message, test.field) {
			t.Fatalf("message: %s", catalog.Message)
		}
	}
}
