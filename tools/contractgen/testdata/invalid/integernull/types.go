package invalid

// +demi:root
type Broken struct {
	// +demi:integer string
	// +demi:nullable
	Value *uint64 `json:"value"`
}
