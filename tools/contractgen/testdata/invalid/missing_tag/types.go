package invalid

// +demi:root
// +demi:union tag=type
type Broken interface{ broken() }

// +demi:variant
type Value struct{}

func (*Value) broken() {}
