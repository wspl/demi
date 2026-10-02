package invalid

// +demi:schema
// +demi:check validate
type Broken string

func validate(Broken) error { return nil }
