package invalid

// +demi:schema
// +demi:check validate
// +demi:root direction=receive
type Broken string

func validate(Broken) error { return nil }
