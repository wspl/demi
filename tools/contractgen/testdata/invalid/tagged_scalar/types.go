package invalid

// +demi:root direction=receive output=web
// +demi:union tag=type
type Broken interface{ broken() }

// +demi:variant value
type BrokenValue string

func (*BrokenValue) broken() {}
