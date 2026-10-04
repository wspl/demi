package types

// What the provider that produced a failure record read out of it when the
// record was shown. Sent beside the transcript, never stored.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type ProviderFailureFacts struct {
	// The moment the vendor says the request can succeed again; null when
	// it names none.
	// +demi:nullable
	RetryAt *Timestamp `json:"retryAt"`
}
