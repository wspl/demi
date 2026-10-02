package grokbuild

import (
	"bytes"
	"context"
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/provider"
)

func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		emitFailure := func(f provider.Failure) {
			if ctx.Err() == nil {
				yield(&provider.Error{Failure: f})
			}
		}
		body, err := provider.EncodeBody(ctx, label, func() ([]byte, error) { return encodeRequest(request) })
		if err != nil {
			var failure *provider.Failure
			if errors.As(err, &failure) {
				emitFailure(*failure)
			}
			return
		}
		var refused *provider.Secret
		for ctx.Err() == nil {
			s, failure := r.shared.auth.credentials(ctx, r.http, refused)
			if failure != nil {
				emitFailure(failure.Failure())
				return
			}
			req, err := http.NewRequestWithContext(ctx, http.MethodPost, r.shared.chatURL.String(), bytes.NewReader(body))
			if err != nil {
				emitFailure(provider.RequestBuildFailure(label, err))
				return
			}
			req.Header = inferenceHeaders(s, request)
			response, err := r.http.Do(req)
			if err != nil {
				emitFailure(provider.TransportFailure(label, err))
				return
			}
			r.shared.quota.Observe(&provider.HTTPObservation{Status: response.StatusCode, Headers: response.Header})
			if response.StatusCode == http.StatusUnauthorized && refused == nil {
				_ = response.Body.Close() // Discard the refused response before the one permitted retry.
				refused = &s.AccessToken
				continue
			}
			defer func() { _ = response.Body.Close() }() // The response is consumed or abandoned; close errors cannot change its result.
			if response.StatusCode < 200 || response.StatusCode >= 300 {
				emitFailure(provider.HTTPFailure(ctx, response, label, provider.ReadHTTPFailure, r.shared.clock))
				return
			}
			for event := range provider.MapChatSSE(ctx, response.Body, provider.Vendor{Label: label, Reader: provider.ReadHTTPFailure, Clock: r.shared.clock}) {
				if ctx.Err() != nil || !yield(event) {
					return
				}
			}
			return
		}
	}
}
