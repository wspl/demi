package types

// A subscription account's public metadata; never token material.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type AccountInfo struct {
	// The account's id within its entry.
	ID string `json:"id"`
	// What the user knows the account by, such as an email address.
	Label string `json:"label"`
	// A second line, such as the plan or the issuer.
	// +demi:nullable
	Detail *string `json:"detail"`
	// When the account was last stored or refreshed.
	// +demi:nullable
	UpdatedAt *Timestamp `json:"updatedAt"`
}

// What a device login asks the user to do, reported once while the login
// waits (`providers.md` § Login and publication).
// +demi:tolerant
// +demi:root direction=receive output=protocol
type LoginPending struct {
	// The address the user opens to confirm the login.
	VerificationURL string `json:"verificationUrl"`
	// The one-time code the user enters there; null when the address
	// carries it.
	// +demi:nullable
	UserCode *string `json:"userCode"`
	// When the code expires, when the vendor says.
	// +demi:nullable
	ExpiresAt *Timestamp `json:"expiresAt"`
}
