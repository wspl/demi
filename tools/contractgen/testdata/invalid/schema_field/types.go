package invalid

// +demi:schema
// +demi:root direction=receive
type Broken struct {
	// +demi:schema
	Value string `json:"value"`
}
