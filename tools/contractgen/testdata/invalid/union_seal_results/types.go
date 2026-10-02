package invalid

// +demi:root
// +demi:union tag=type
type Broken interface {
	ID() string
	seal() bool
}

// +demi:variant Broken value
type Value struct{}
