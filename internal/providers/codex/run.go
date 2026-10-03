package codex

import (
	"context"
	"errors"
	"iter"
	"net/http"

	"github.com/wspl/demi/internal/provider"
)

// Run streams inference events for the request.
func (s *session) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		if ctx.Err() != nil {
			return
		}
		body, err := provider.EncodeBody(ctx, "Codex", func() ([]byte, error) { return encodeRequest(request) })
		if err != nil {
			emitEncodingFailure(ctx, err, yield)
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
				if s.openFailure(err, &refused, credentials, yield) {
					continue
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

// openFailure returns true when a first unauthorized response requires credential refresh.
func (s *session) openFailure(
	err error,
	refused **provider.Secret,
	credentials secret,
	yield func(provider.Event) bool,
) bool {
	var rejection *refusalError
	if errors.As(err, &rejection) {
		if rejection.status == http.StatusUnauthorized && *refused == nil {
			*refused = &credentials.AccessToken
			return true
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
	return false
}

func emitEncodingFailure(ctx context.Context, err error, yield func(provider.Event) bool) {
	if ctx.Err() != nil {
		return
	}
	var f *provider.Failure
	if errors.As(err, &f) {
		yield(&provider.Error{Failure: *f})
	}
}
