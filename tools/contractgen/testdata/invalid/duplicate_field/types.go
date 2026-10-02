package invalid

// +demi:strict
// +demi:root direction=receive
type Broken struct {
	A string `json:"a"`
	B string `json:"a"`
}
