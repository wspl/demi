package invalid

// +demi:root
// +demi:union tag=type
type Broken interface {
	ID() string
	first()
	second()
}

// +demi:variant Broken value
type Value struct{}
