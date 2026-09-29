package core

//demi:wire open
type AccountInfo struct {
	ID        string     `json:"id"`
	Label     string     `json:"label"`
	Detail    *string    `json:"detail" check:"nullable"`
	UpdatedAt *Timestamp `json:"updatedAt" check:"nullable,func=Validate"`
}

//demi:wire open
type LoginPending struct {
	VerificationURL string     `json:"verificationUrl"`
	UserCode        *string    `json:"userCode" check:"nullable"`
	ExpiresAt       *Timestamp `json:"expiresAt" check:"nullable,func=Validate"`
}

//demi:enum
type WireAPI string

const (
	WireAPIResponses       WireAPI = "responses"
	WireAPIChatCompletions WireAPI = "chat-completions"
)

//demi:union tag=status
type AuthState interface{ isAuthState() }

//demi:variant unknown open
type AuthStateUnknown struct {
	Message *string `json:"message,omitzero"`
}

func (AuthStateUnknown) isAuthState() {}

//demi:variant authenticated open
type AuthStateAuthenticated struct {
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

//demi:union tag=status
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
