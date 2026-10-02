package invalid

// +demi:strict
// +demi:root direction=receive
type Broken struct {
	// +demi:nullable
	Value *string `json:"value,omitempty"`
}
