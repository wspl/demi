package core

// ProviderFailureFacts holds facts a provider extracts when a stored failure is displayed.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type ProviderFailureFacts struct {
	// +demi:nullable
	RetryAt *Timestamp `json:"retryAt"`
}
