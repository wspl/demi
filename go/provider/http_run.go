package provider

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"iter"
	"net/http"
)

// HTTPRun posts a vendor's JSON body and owns its response until the mapper
// ends, the consumer stops, or the context is cancelled.
func HTTPRun(ctx context.Context, client *http.Client, endpoint string, headers http.Header, body func() []byte, vendor Vendor, mapBody func(context.Context, io.Reader) iter.Seq[ProviderEvent]) iter.Seq[ProviderEvent] {
	return func(yield func(ProviderEvent) bool) {
		if ctx.Err() != nil {
			return
		}
		encoded, failure := EncodeBody(vendor.Label, body)
		if ctx.Err() != nil {
			return
		}
		if failure != nil {
			yield(FailureEvent{Failure: *failure})
			return
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(encoded))
		if err != nil {
			yield(FailureEvent{Failure: ProviderFailure{Message: vendor.Label + " API request could not be built"}})
			return
		}
		request.Header = headers.Clone()
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Accept", "text/event-stream")
		response, err := client.Do(request)
		if ctx.Err() != nil {
			if response != nil {
				response.Body.Close()
			}
			return
		}
		if err != nil {
			yield(FailureEvent{Failure: TransportFailure(vendor.Label, err)})
			return
		}
		defer response.Body.Close()
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			failure := HTTPFailure(response, vendor.Label, vendor.Reader, vendor.Clock)
			if ctx.Err() == nil {
				yield(FailureEvent{Failure: failure})
			}
			return
		}
		for event := range mapBody(ctx, response.Body) {
			if ctx.Err() != nil || !yield(event) {
				return
			}
		}
	}
}

// EncodeBody contains a failed body build within its inference attempt.
func EncodeBody(label string, build func() []byte) (body []byte, failure *ProviderFailure) {
	defer func() {
		if recovered := recover(); recovered != nil {
			body = nil
			failure = &ProviderFailure{Message: fmt.Sprintf("%s API request body was not built", label)}
		}
	}()
	return build(), nil
}
