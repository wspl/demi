package invalid

// +demi:root
// +demi:union tag=op content=op
type Broken interface{ seal() }

// +demi:variant Broken yes
type Yes struct{}
