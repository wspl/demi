package invalid

type Base struct {
	Value string `json:"value"`
}

// +demi:strict
type Broken struct {
	// +demi:unknown
	Base
}
