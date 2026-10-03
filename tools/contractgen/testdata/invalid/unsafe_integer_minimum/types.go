package invalid

// +demi:root direction=receive output=protocol
type Broken struct {
	// +demi:range max=5
	Value int64 `json:"value"`
}
