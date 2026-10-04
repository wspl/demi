package usershard

import (
	"context"
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) testProvider(
	ctx context.Context,
	entry providers.Entry,
	builtProvider provider.Provider,
	account *webapi.CredentialID,
	modelID string,
) (webapi.TestResult, error) {
	catalog := s.services.Assembly.EntryCatalog(ctx, entry, builtProvider, nil, false)
	for _, model := range catalog.Models {
		if model.ID != modelID {
			continue
		}
		failed := func(message string) webapi.TestResult {
			return &webapi.TestResultFailed{Message: message, Model: &model.DisplayName}
		}
		requested := model.Selection(string(entry.ID), nil, nil)
		selection, err := providers.ConfiguredSelection(entry, requested)
		if err != nil {
			return failed(err.Error()), nil
		}
		var runtime provider.Runtime
		if builtProvider.Capabilities().ProcessHost {
			if account == nil {
				account = entry.Active()
			}
			runtime, err = s.services.Assembly.ProcessRuntime(
				ctx,
				entry,
				account,
				cloudPlacement{shard: s, entry: entry.ID},
			)
		} else {
			runtime, err = builtProvider.Runtime(provider.RuntimeEnv{HTTP: s.http})
		}
		if err != nil {
			return failed(err.Error()), nil
		}
		first := testInference(ctx, runtime, model.ID, selection.Model.OutputLimit)
		if ctx.Err() != nil {
			return failed("The test was cancelled"), nil
		}
		if first == nil {
			return failed("The provider returned no events"), nil
		}
		if failure, ok := first.(*provider.Error); ok {
			return failed(failure.Failure.Message), nil
		}
		return &webapi.TestResultPassed{Model: model.DisplayName}, nil
	}
	return &webapi.TestResultFailed{
		Message: fmt.Sprintf("This provider lists no model %s", modelID),
	}, nil
}

// testInference sends the minimal provider test request and closes its runtime.
func testInference(
	ctx context.Context,
	runtime provider.Runtime,
	modelID string,
	outputLimit *uint32,
) provider.Event {
	requestCtx, cancel := context.WithCancel(ctx)
	ids := transcript.RandomIDs{}
	request := provider.InferenceRequest{
		SessionID:    "provider-test",
		TurnID:       ids.NextID(),
		RequestID:    ids.NextID(),
		ModelID:      modelID,
		OutputLimit:  outputLimit,
		SystemPrompt: "Reply with the word ok.",
		Items: []provider.InferenceItem{
			&provider.UserMessage{
				Content: []provider.UserPart{&provider.TextPart{Text: "ping"}},
			},
		},
		Tools: []provider.ToolDefinition{},
	}
	var first provider.Event
	for event := range runtime.Run(requestCtx, request) {
		first = event
		break
	}
	cancel()
	if err := runtime.Close(context.WithoutCancel(ctx)); err != nil {
		slog.Warn("the provider test runtime did not close", "error", err)
	}
	return first
}
