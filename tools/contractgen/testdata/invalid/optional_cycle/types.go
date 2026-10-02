package invalid

// +demi:root
type Broken struct {
	*Broken
}
