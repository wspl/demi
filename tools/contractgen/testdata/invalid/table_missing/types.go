package invalid

// +demi:table
var Broken = []struct {
	Value string `json:"value"`
}{{}}
