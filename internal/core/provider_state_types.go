package core

// AccountInfo is public subscription-account metadata, without credentials.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type AccountInfo struct {
	ID    string `json:"id"`
	Label string `json:"label"`
	// +demi:nullable
	Detail *string `json:"detail"`
	// +demi:nullable
	UpdatedAt *Timestamp `json:"updatedAt"`
}

// LoginPending describes the action a device login requires.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type LoginPending struct {
	VerificationURL string `json:"verificationUrl"`
	// +demi:nullable
	UserCode *string `json:"userCode"`
	// +demi:nullable
	ExpiresAt *Timestamp `json:"expiresAt"`
}
