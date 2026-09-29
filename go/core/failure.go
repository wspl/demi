package core

// The diagnostics of a provider failure, saved with its `error` block. The
// vendor's answer is in `upstream` exactly as it arrived: a stream's frame
// text, or the JSON `{ status, headers, body }` of an HTTP failure; only the
// provider that produced it reads it.
//
//demi:wire
type ProviderErrorDiagnostics struct {
	Source             FailureSource `json:"source"`
	ClientRequestID    *string       `json:"clientRequestId,omitzero"`
	ProviderRequestID  *string       `json:"providerRequestId,omitzero"`
	ProviderResponseID *string       `json:"providerResponseId,omitzero"`
	ProviderCode       *string       `json:"providerCode,omitzero"`
	HTTPStatus         *uint16       `json:"httpStatus,omitzero"`
	Upstream           *string       `json:"upstream,omitzero"`
}

// What the provider that produced a failure record read out of it when the
// record was shown. Sent beside the transcript, never stored.
//
//demi:wire open
type ProviderFailureFacts struct {
	// The moment the vendor says the request can succeed again; null when
	// it names none.
	RetryAt *Timestamp `json:"retryAt" check:"nullable,func=Validate"`
}

// Where a provider failure came from.
//
//demi:enum
//demi:export
type FailureSource string

const (
	FailureSourceUnknown   FailureSource = "unknown"
	FailureSourceHTTP      FailureSource = "http"
	FailureSourceStream    FailureSource = "stream"
	FailureSourceTransport FailureSource = "transport"
)
