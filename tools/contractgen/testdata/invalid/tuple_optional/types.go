package invalid

// +demi:msgpack tuple
// +demi:union tag=type
type Record interface{ record() }

// +demi:variant Record output
type Broken struct {
	Value *string `json:"value,omitempty"`
}
