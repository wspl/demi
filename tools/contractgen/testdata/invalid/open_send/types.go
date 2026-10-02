package invalid

// +demi:root direction=send output=web
// +demi:tolerant
type Broken struct {
	A string `json:"a"`
}
