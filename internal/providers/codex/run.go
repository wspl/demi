package codex

import (
	"context"
	"errors"
	"iter"
	"net/http"

	"github.com/wspl/demi/internal/provider"
)

func (s *session) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		if ctx.Err() != nil {
			return
		}
		body, err := provider.EncodeBody(ctx, "Codex", func() ([]byte, error) { return encodeRequest(request) })
		if err != nil {
			if ctx.Err() == nil {
				var f *provider.Failure
				if errors.As(err, &f) {
					yield(&provider.Error{Failure: *f})
				}
			}
			return
		}
		var refused *provider.Secret
		for {
			credentials, err := s.p.credentials(ctx, s.http, refused)
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				yield(&provider.Error{Failure: authFailure(err).Failure()})
				return
			}
			events, cleanup, err := s.p.open(ctx, s.http, inferenceHeaders(credentials, request), body)
			if ctx.Err() != nil {
				if cleanup != nil {
					cleanup()
				}
				return
			}
			if err != nil {
				var rejection *refusal
				if errors.As(err, &rejection) {
					if rejection.status == http.StatusUnauthorized && refused == nil {
						refused = &credentials.AccessToken
						continue
					}
					yield(&provider.Error{Failure: s.p.refused(rejection)})
				} else {
					var f *provider.Failure
					if errors.As(err, &f) {
						yield(&provider.Error{Failure: *f})
					} else {
						yield(&provider.Error{Failure: provider.TransportFailure("Codex", err)})
					}
				}
				return
			}
			defer cleanup()
			vendor := provider.Vendor{Label: "Codex", Reader: readFailure, Clock: s.p.clock}
			provider.MapResponsesEvents(ctx, events, vendor, "codex:")(yield)
			return
		}
	}
}

type events = iter.Seq2[provider.Received, error]
