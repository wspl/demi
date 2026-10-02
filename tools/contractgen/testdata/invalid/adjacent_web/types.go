package invalid

// +demi:root direction=receive output=web
// +demi:union tag=op content=result
type Broken interface{ seal() }

// +demi:variant Broken yes
type Yes struct{}
