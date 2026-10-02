package invalid

// +demi:root
// +demi:union untagged
type Broken interface{ broken() }

// +demi:variant
// +demi:tolerant
type Open struct{}

func (*Open) broken() {}
