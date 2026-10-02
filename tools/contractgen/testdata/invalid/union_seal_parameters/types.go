package invalid

// +demi:root
// +demi:union tag=type
type Broken interface {
	ID() string
	seal(string)
}

// +demi:variant Broken value
type Value struct{}
