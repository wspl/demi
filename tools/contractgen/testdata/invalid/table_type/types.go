package invalid

// +demi:table
// +demi:root direction=receive
type Broken struct {
	Value string `json:"value"`
}
