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

func (s *Shard) testProvider(ctx context.Context, entry providers.ProviderEntry, p provider.Provider, account *webapi.CredentialID, modelID string) (webapi.TestResult, error) {
	catalog := s.services.Assembly.EntryCatalog(ctx, entry, p, nil, false)
	for _, model := range catalog.Models {
		if model.ID != modelID {
			continue
		}
		failed := func(message string) webapi.TestResult {
			return &webapi.TestResultFailed{Message: message, Model: &model.DisplayName}
		}
		selection, err := providers.ConfiguredSelection(entry, model.Selection(string(entry.ID), nil, nil))
		if err != nil {
			return failed(err.Error()), nil
		}
		var runtime provider.Runtime
		if p.Capabilities().ProcessHost {
			if account == nil {
				account = entry.Active()
			}
			runtime, err = s.services.Assembly.ProcessRuntime(ctx, entry, account, cloudPlacement{shard: s, entry: entry.ID})
		} else {
			runtime, err = p.Runtime(provider.RuntimeEnv{HTTP: s.http})
		}
		if err != nil {
			return failed(err.Error()), nil
		}
		requestCtx, cancel := context.WithCancel(ctx)
		ids := transcript.RandomIDs{}
		request := provider.InferenceRequest{SessionID: "provider-test", TurnID: ids.NextID(), RequestID: ids.NextID(), ModelID: model.ID, OutputLimit: selection.Model.OutputLimit, SystemPrompt: "Reply with the word ok.", Items: []provider.InferenceItem{&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "ping"}}}}, Tools: []provider.ToolDefinition{}}
		var first provider.Event
		for event := range runtime.Run(requestCtx, request) {
			first = event
			break
		}
		cancel()
		if err := runtime.Close(context.WithoutCancel(ctx)); err != nil {
			slog.Warn("the provider test runtime did not close", "error", err)
		}
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
	return &webapi.TestResultFailed{Message: fmt.Sprintf("This provider lists no model %s", modelID)}, nil
}
