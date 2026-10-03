package openaiapi

import (
	"bytes"
	"context"
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

type runtime struct {
	shared *Provider
	http   *http.Client
}

// Fresh returns an independent runtime for another session.
func (r *runtime) Fresh() provider.Runtime { return &runtime{shared: r.shared, http: r.http} }

// Close releases the runtime resources.
func (*runtime) Close(context.Context) error { return nil }

// RequestLimits returns the request limits for the model.
func (*runtime) RequestLimits(core.Model) provider.RequestLimits {
	return provider.OpenAIRequestLimits()
}

// Run streams inference events for the request.
func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		if ctx.Err() != nil {
			return
		}
		body, err := provider.EncodeBody(ctx, "OpenAI", func() ([]byte, error) {
			if r.shared.wire == core.WireAPIChatCompletions {
				return chatBody(request, r.shared.policy)
			}
			return responsesBody(request, r.shared.policy)
		})
		if ctx.Err() != nil {
			return
		}
		if err != nil {
			var failure *provider.Failure
			if errors.As(err, &failure) {
				yield(&provider.Error{Failure: *failure})
			}
			return
		}
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, r.shared.url.String(), bytes.NewReader(body))
		if err != nil {
			yield(&provider.Error{Failure: provider.RequestBuildFailure("OpenAI", err)})
			return
		}
		req.Header.Set("Authorization", r.shared.authorization.Expose())
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Accept", "text/event-stream")
		response, err := r.http.Do(req)
		if err != nil {
			if ctx.Err() == nil {
				yield(&provider.Error{Failure: provider.TransportFailure("OpenAI", err)})
			}
			return
		}
		defer func() {
			_ = response.Body.Close() // Closing releases the connection; read errors are reported by the mapper.
		}()
		if ctx.Err() != nil {
			return
		}
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			failure := provider.HTTPFailure(ctx, response, "OpenAI", provider.ReadHTTPFailure, r.shared.clock)
			if ctx.Err() == nil {
				yield(&provider.Error{Failure: failure})
			}
			return
		}
		r.stream(ctx, response, yield)
	}
}

func (r *runtime) stream(ctx context.Context, response *http.Response, yield func(provider.Event) bool) {
	vendor := provider.Vendor{Label: "OpenAI", Reader: provider.ReadHTTPFailure, Clock: r.shared.clock}
	var events provider.Run
	if r.shared.wire == core.WireAPIChatCompletions {
		events = provider.MapChatSSE(ctx, response.Body, vendor)
	} else {
		events = provider.MapResponsesEvents(
			ctx,
			provider.ResponsesSSEEvents(ctx, response.Body, "OpenAI"),
			vendor,
			signatureTag,
		)
	}
	for event := range events {
		if ctx.Err() != nil || !yield(event) {
			return
		}
	}
}
