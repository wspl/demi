package invalid

// +demi:union tag=op content=result
type Reply interface{ seal() }

// +demi:root direction=receive output=web
// +demi:variant Reply yes
type Broken struct{}
