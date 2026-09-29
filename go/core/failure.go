package core

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

//demi:wire open
type ProviderFailureFacts struct {
	RetryAt *Timestamp `json:"retryAt" check:"nullable,func=Validate"`
}

//demi:enum
type FailureSource string

const (
	FailureSourceHTTP      FailureSource = "http"
	FailureSourceStream    FailureSource = "stream"
	FailureSourceTransport FailureSource = "transport"
	FailureSourceUnknown   FailureSource = "unknown"
)
