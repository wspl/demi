package invalid

// +demi:root direction=receive
type Broken struct {
	Value chan string `json:"value"`
}
