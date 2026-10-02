package invalid

// +demi:root
type Broken struct {
	Op string `json:"op"`
	// +demi:flatten
	Result Reply
}

// +demi:union tag=op content=result
type Reply interface{ seal() }

// +demi:variant Reply yes
type Yes struct{}
