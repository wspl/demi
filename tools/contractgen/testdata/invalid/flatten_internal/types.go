package invalid

// +demi:root
type Broken struct {
	// +demi:flatten
	Result Reply
}

// +demi:union tag=op
type Reply interface{ seal() }

// +demi:variant Reply yes
type Yes struct{}
