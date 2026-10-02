package invalid

// +demi:root
type Broken struct {
	// +demi:default
	Value *string `json:"value"`
}
