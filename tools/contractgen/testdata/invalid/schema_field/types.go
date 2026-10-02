package invalid

// +demi:schema
type Broken struct {
	// +demi:schema
	Value string `json:"value"`
}
