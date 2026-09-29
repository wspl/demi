package anthropicapi_test

import (
	"encoding/json/v2"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider/providertest"
)

// Cost: seven loopback requests on one runtime; no model calls.
func TestThinkingModesAndPerRequestLimits(t *testing.T) {
	vendor := providertest.NewMockVendor(t)
	runtime := runtimeAt(t, vendor)
	for _, tc := range []struct {
		name                  string
		thinking              core.ThinkingConfig
		limit                 *uint32
		wantLimit             uint32
		mode, display, effort string
		budget                uint32
	}{
		{name: "default", wantLimit: 32000},
		{name: "disabled", thinking: core.ThinkingConfigDisabled{}, limit: new(uint32(100)), wantLimit: 100},
		{name: "minimum", thinking: core.ThinkingConfigBudget{BudgetTokens: 1}, limit: new(uint32(2000)), wantLimit: 2000, mode: "enabled", budget: 1024},
		{name: "ceiling", thinking: core.ThinkingConfigBudget{BudgetTokens: 99999}, limit: new(uint32(6000)), wantLimit: 6000, mode: "enabled", budget: 4976},
		{name: "adaptive", thinking: core.ThinkingConfigAdaptive{Effort: "max"}, wantLimit: 32000, mode: "adaptive", display: "summarized", effort: "max"},
		{name: "effort", thinking: core.ThinkingConfigEffort{Effort: "low"}, wantLimit: 32000, mode: "adaptive", display: "summarized", effort: "low"},
		{name: "hidden", thinking: core.ThinkingConfigEffort{Effort: "high", Summary: new(core.ThinkingSummaryOff)}, wantLimit: 32000, mode: "adaptive", display: "omitted", effort: "high"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			vendor.Route("/v1/messages", providertest.MockResponse{})
			request := providertest.InferenceRequest()
			request.Thinking = tc.thinking
			request.OutputLimit = tc.limit
			providertest.Run(t, runtime, request)
			requests := vendor.Requests()
			var body struct {
				MaxTokens uint32 `json:"max_tokens"`
				Thinking  *struct {
					Type    string `json:"type"`
					Display string `json:"display"`
					Budget  uint32 `json:"budget_tokens"`
				} `json:"thinking"`
				Output *struct {
					Effort string `json:"effort"`
				} `json:"output_config"`
			}
			if err := json.Unmarshal(requests[len(requests)-1].Body, &body); err != nil {
				t.Fatal(err)
			}
			if body.MaxTokens != tc.wantLimit {
				t.Fatalf("limit %d, want %d", body.MaxTokens, tc.wantLimit)
			}
			if tc.mode == "" {
				if body.Thinking != nil || body.Output != nil {
					t.Fatalf("unexpected thinking: %s", requests[len(requests)-1].Body)
				}
				return
			}
			if body.Thinking == nil || body.Thinking.Type != tc.mode || body.Thinking.Display != tc.display || body.Thinking.Budget != tc.budget {
				t.Fatalf("thinking: %s", requests[len(requests)-1].Body)
			}
			if tc.effort == "" {
				if body.Output != nil {
					t.Fatal("budget has output_config")
				}
			} else if body.Output == nil || body.Output.Effort != tc.effort {
				t.Fatalf("effort: %s", requests[len(requests)-1].Body)
			}
		})
	}
}
