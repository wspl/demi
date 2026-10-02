package invalid

// +demi:table foo
var Broken = []struct {
	Value string `json:"value"`
}{{"x"}}
