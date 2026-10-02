package kept

//go:generate go run ../..

// +demi:union tag=type
// +demi:msgpack tuple
// +demi:root
type Record interface{ record() }

// +demi:enum stdout stderr
type Stream string

// +demi:variant Record output
type Output struct {
	Stream Stream `json:"stream"`
	Data   []byte `json:"data"`
}

// +demi:variant Record left_out
type LeftOut struct {
	Bytes uint64 `json:"bytes"`
}
