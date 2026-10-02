package invalid

// +demi:union tag=type
type Union interface{ isUnion() }

// +demi:variant value
// +demi:root direction=receive
type Broken struct{}

func (Broken) isUnion() {}
