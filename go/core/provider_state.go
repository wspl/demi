package core

// A subscription account's public metadata; never token material.
//
//demi:wire open
type AccountInfo struct {
	// The account's id within its entry.
	ID string `json:"id"`
	// What the user knows the account by, such as an email address.
	Label string `json:"label"`
	// A second line, such as the plan or the issuer.
	Detail *string `json:"detail" check:"nullable"`
	// When the account was last stored or refreshed.
	UpdatedAt *Timestamp `json:"updatedAt" check:"nullable,func=Validate"`
}

// What a device login asks the user to do, reported once while the login
// waits (`providers.md` § Login and publication).
//
//demi:wire open
type LoginPending struct {
	// The address the user opens to confirm the login.
	VerificationURL string `json:"verificationUrl"`
	// The one-time code the user enters there; null when the address
	// carries it.
	UserCode *string `json:"userCode" check:"nullable"`
	// When the code expires, when the vendor says.
	ExpiresAt *Timestamp `json:"expiresAt" check:"nullable,func=Validate"`
}

// The wire format an `openai` entry speaks (`providers.md` § Endpoints): the
// Responses API, or Chat Completions as OpenAI-compatible vendors serve it.
// An entry that names none speaks Responses.
//
//demi:enum
//demi:export
type WireAPI string

const (
	WireAPIResponses       WireAPI = "responses"
	WireAPIChatCompletions WireAPI = "chat-completions"
)

// Whether a provider's credential is present and usable. Reading it never
// makes an inference request.
//
//demi:union tag=status
//demi:export
type AuthState interface{ isAuthState() }

//demi:variant unknown open
type AuthStateUnknown struct {
	Message *string `json:"message,omitzero"`
}

func (AuthStateUnknown) isAuthState() {}

//demi:variant authenticated open
type AuthStateAuthenticated struct {
	// The account the credential acts for, when the provider can name
	// it.
	AccountLabel *string `json:"accountLabel,omitzero"`
}

func (AuthStateAuthenticated) isAuthState() {}

//demi:variant unauthenticated open
type AuthStateUnauthenticated struct {
	Message *string `json:"message,omitzero"`
}

func (AuthStateUnauthenticated) isAuthState() {}

//demi:variant error open
type AuthStateError struct {
	Message string `json:"message"`
}

func (AuthStateError) isAuthState() {}

// Whether a provider can run requests at all.
//
//demi:union tag=status
//demi:export
type RuntimeState interface{ isRuntimeState() }

//demi:variant unknown open
type RuntimeStateUnknown struct {
	Message *string `json:"message,omitzero"`
}

func (RuntimeStateUnknown) isRuntimeState() {}

//demi:variant ready open
type RuntimeStateReady struct {
	Message *string `json:"message,omitzero"`
}

func (RuntimeStateReady) isRuntimeState() {}

//demi:variant unavailable open
type RuntimeStateUnavailable struct {
	Message string `json:"message"`
}

func (RuntimeStateUnavailable) isRuntimeState() {}

//demi:variant error open
type RuntimeStateError struct {
	Message string `json:"message"`
}

func (RuntimeStateError) isRuntimeState() {}
