package invalid

// +demi:union tag=type
type Union interface{ isUnion() }

// +demi:variant value
type Broken struct{}

func (Broken) isUnion() {}
