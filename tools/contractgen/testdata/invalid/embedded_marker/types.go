package invalid

type Base struct {
	Value string `json:"value"`
}

// +demi:strict
// +demi:root direction=receive
type Broken struct {
	// +demi:unknown
	Base
}
