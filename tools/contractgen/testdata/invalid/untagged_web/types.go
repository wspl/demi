package invalid

// +demi:root direction=receive output=web
// +demi:union untagged
type Broken interface{ broken() }

// +demi:variant
type Value struct {
	Text string `json:"text"`
}

func (*Value) broken() {}
