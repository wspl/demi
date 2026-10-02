package invalid

// +demi:root
type Broken struct {
	// +demi:codec
	Value string `json:"value"`
}
