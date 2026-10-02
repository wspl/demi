package invalid

// +demi:strict
type Broken struct {
	A string `json:"a"`
	B string `json:"a"`
}
