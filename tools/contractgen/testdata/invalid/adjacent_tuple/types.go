package invalid

// +demi:root
// +demi:msgpack tuple
// +demi:union tag=op content=result
type Broken interface{ seal() }

// +demi:variant Broken yes
type Yes struct{}
