package invalid

// +demi:root
type Broken struct {
	// +demi:schema-primitive
	Value string `json:"value"`
}
