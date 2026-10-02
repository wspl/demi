package invalid

// +demi:strict
type Broken struct {
	// +demi:nullable
	Value *string `json:"value,omitempty"`
}
