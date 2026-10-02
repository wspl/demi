package invalid

// +demi:root direction=receive output=protocol
type Broken struct {
	Value uint64 `json:"value"`
}
