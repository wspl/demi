package invalid

// +demi:schema
type Broken struct {
	Next *Broken `json:"next,omitempty"`
}
