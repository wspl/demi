package invalid

// +demi:integer string
type ID uint64

// +demi:root
type Broken struct {
	// +demi:nullable
	Value *ID `json:"value"`
}
