package invalid

// +demi:root direction=send
type Broken struct {
	// +demi:format trimmed
	Name string `json:"name"`
}
