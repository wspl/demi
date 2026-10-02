package invalid

// +demi:root
type Broken struct {
	// +demi:default true
	Value string `json:"value"`
}
