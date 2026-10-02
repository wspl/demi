package invalid

// +demi:root
// +demi:union tag=op content=result
type Reply interface{ seal() }

// +demi:variant Reply nested
type Broken struct {
	// +demi:flatten
	Value Inner
}

// +demi:union tag=kind content=value
type Inner interface{ inner() }

// +demi:variant Inner empty
type Empty struct{}
