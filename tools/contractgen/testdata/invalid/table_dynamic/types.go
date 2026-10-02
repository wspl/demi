package invalid

func value() string { return "x" }

// +demi:table
var Broken = []struct {
	Value string `json:"value"`
}{{value()}}
