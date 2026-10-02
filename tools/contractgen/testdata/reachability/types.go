package reachability

//go:generate go run ../.. .

// +demi:root
type JSONOnly struct {
	Shared Shared `json:"shared"`
}

// +demi:msgpack
type Packed struct {
	Shared Shared `json:"shared"`
	Choice Choice `json:"choice"`
}

type Shared struct {
	Text string `json:"text"`
}

// +demi:union tag=kind
type Choice interface{ choice() }

// +demi:variant Choice item
type Item struct {
	// +demi:range min=1 max=10
	Number int32 `json:"number"`
}

// +demi:msgpack
// +demi:union tag=kind
type Standalone interface{ standalone() }

// +demi:variant Standalone empty
type Empty struct{}

// Unsupported runtime declarations outside both graphs remain untouched.
type Unreached chan int
